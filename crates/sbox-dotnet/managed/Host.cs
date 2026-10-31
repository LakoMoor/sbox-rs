using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

namespace SboxHost;

public static unsafe class Host
{
    static readonly List<Type> Pages = new();
    static readonly List<object> Instances = new();
    static string last = "";

    static string Str(byte* p, int n) => Encoding.UTF8.GetString(p, n);

    static unsafe int Put(string s, byte* o, int cap)
    {
        var b = Encoding.UTF8.GetBytes(s);
        if (b.Length > cap) return -b.Length;
        b.CopyTo(new Span<byte>(o, cap));
        return b.Length;
    }

    /// <summary>Compiles the members of a Razor `@code` block into a class; returns its id or -1 (see LastError).</summary>
    [UnmanagedCallersOnly]
    public static unsafe int Load(byte* code, int len)
    {
        try
        {
            var src = "using System; using System.Collections.Generic; using System.Linq;\n" +
                      "public static class Log { public static List<string> Lines = new(); public static void Info(object o) => Lines.Add(\"info: \" + o); public static void Warning(object o) => Lines.Add(\"warn: \" + o); public static void Error(object o) => Lines.Add(\"error: \" + o); }\n" +
                      "public class PageCode {\n" + Str(code, len) + "\n}";
            var refs = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator)
                .Select(p => MetadataReference.CreateFromFile(p)).Cast<MetadataReference>().ToList();
            refs.Add(MetadataReference.CreateFromFile(typeof(Host).Assembly.Location));
            var comp = CSharpCompilation.Create("page" + Pages.Count,
                new[] { CSharpSyntaxTree.ParseText(src) }, refs,
                new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, nullableContextOptions: NullableContextOptions.Disable));
            using var ms = new MemoryStream();
            var res = comp.Emit(ms);
            if (!res.Success)
            {
                last = string.Join("\n", res.Diagnostics.Where(d => d.Severity == DiagnosticSeverity.Error).Select(d => d.ToString()));
                return -1;
            }
            var t = Assembly.Load(ms.ToArray()).GetType("PageCode")!;
            Pages.Add(t);
            Instances.Add(Activator.CreateInstance(t)!);
            return Pages.Count - 1;
        }
        catch (Exception e) { last = e.ToString(); return -1; }
    }

    /// <summary>Sets values (JSON object) on the page, optionally calls `method`, returns all public state and Log output as JSON.</summary>
    [UnmanagedCallersOnly]
    public static unsafe int Call(int id, byte* method, int mlen, byte* values, int vlen, byte* outp, int cap)
    {
        try
        {
            var t = Pages[id];
            var o = Instances[id];
            var lines = (List<string>)t.Assembly.GetType("Log")!.GetField("Lines")!.GetValue(null)!;
            lines.Clear();
            foreach (var p in JsonDocument.Parse(Str(values, vlen)).RootElement.EnumerateObject())
                Set(t, o, p.Name, p.Value);
            var m = Str(method, mlen);
            if (m.Length > 0) t.GetMethod(m, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance)!.Invoke(o, null);
            var state = new Dictionary<string, object?>();
            foreach (var f in t.GetFields(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance))
                if (!f.Name.Contains('<') && Simple(f.FieldType)) state[f.Name] = f.GetValue(o);
            foreach (var p in t.GetProperties(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance))
                if (p.GetIndexParameters().Length == 0 && p.CanRead && Simple(p.PropertyType)) state[p.Name] = p.GetValue(o);
            return Put(JsonSerializer.Serialize(new { state, log = lines }), outp, cap);
        }
        catch (Exception e) { last = (e.InnerException ?? e).ToString(); return int.MinValue; }
    }

    [UnmanagedCallersOnly]
    public static unsafe int LastError(byte* outp, int cap) => Put(last, outp, cap);

    static bool Simple(Type t) => t.IsPrimitive || t == typeof(string) || t == typeof(decimal);

    static void Set(Type t, object o, string name, JsonElement v)
    {
        var f = t.GetField(name, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance);
        if (f != null) { f.SetValue(o, Conv(v, f.FieldType)); return; }
        var p = t.GetProperty(name, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance);
        if (p != null && p.CanWrite) p.SetValue(o, Conv(v, p.PropertyType));
    }

    static object? Conv(JsonElement v, Type t) =>
        t == typeof(string) ? (v.ValueKind == JsonValueKind.String ? v.GetString() : v.ToString())
        : t == typeof(bool) ? (v.ValueKind == JsonValueKind.String ? v.GetString() == "true" : v.GetBoolean())
        : Convert.ChangeType(v.ValueKind == JsonValueKind.String ? double.Parse(v.GetString()!, System.Globalization.CultureInfo.InvariantCulture) : v.GetDouble(), t);
}
