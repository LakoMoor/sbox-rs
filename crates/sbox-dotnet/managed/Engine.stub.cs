// A small stand-in for the s&box engine API that Razor pages call. Only what the host can honour: no scene, no menu services.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;

[AttributeUsage(AttributeTargets.All, AllowMultiple = true)] public class ParameterAttribute : Attribute { }
[AttributeUsage(AttributeTargets.All, AllowMultiple = true)] public class PropertyAttribute : Attribute { public PropertyAttribute() { } public PropertyAttribute(string n) { } }
[AttributeUsage(AttributeTargets.All, AllowMultiple = true)] public class TitleAttribute : Attribute { public TitleAttribute(string n) { } }
[AttributeUsage(AttributeTargets.All, AllowMultiple = true)] public class GroupAttribute : Attribute { public GroupAttribute(string n) { } }
[AttributeUsage(AttributeTargets.All, AllowMultiple = true)] public class RangeAttribute : Attribute { public RangeAttribute(float a, float b) { } }
[AttributeUsage(AttributeTargets.All, AllowMultiple = true)] public class SyncAttribute : Attribute { public SyncAttribute() { } public SyncAttribute(object o) { } }
[AttributeUsage(AttributeTargets.All, AllowMultiple = true)] public class FromHostAttribute : Attribute { }

public static class Time { public static float Now => (float)Environment.TickCount64 / 1000f; public static float Delta => 1f / 60f; }
public static class RealTime { public static float Now => (float)Environment.TickCount64 / 1000f; public static float Delta => 1f / 60f; }

public struct RealTimeSince
{
    float t0;
    public static implicit operator RealTimeSince(float ago) => new RealTimeSince { t0 = RealTime.Now - ago };
    public static implicit operator float(RealTimeSince s) => RealTime.Now - s.t0;
    public float Relative => RealTime.Now - t0;
}

public struct RealTimeUntil
{
    float t1;
    public static implicit operator RealTimeUntil(float left) => new RealTimeUntil { t1 = RealTime.Now + left };
    public static implicit operator float(RealTimeUntil s) => s.t1 - RealTime.Now;
    public float Relative => t1 - RealTime.Now;
    public bool Passed => Relative <= 0;
}

public static class Game
{
    public static bool InGame => false;
    public static bool IsPlaying => false;
    public static bool IsEditor => false;
    public static bool IsClosing => false;
    public static void Close() { }
}

public static class Input
{
    public static bool Down(string action) => false;
    public static bool Pressed(string action) => false;
    public static bool Released(string action) => false;
}

public class Scene { public bool IsValid => false; }

public class MousePanelEvent { public string Button = ""; public Panel Target; public void StopPropagation() { } }
public class PanelEvent { public Panel Target; public void StopPropagation() { } }
public class RenderFragment { }

public class Panel
{
    public Panel Parent;
    public bool IsVisible => true;
    public string Id = "";
    public Dictionary<string, bool> Classes = new();
    public void SetClass(string name, bool on) => Classes[name] = on;
    public void StateHasChanged() { }
    public void Delete() { }
    public bool HasClass(string name) => Classes.TryGetValue(name, out var b) && b;
    public virtual void OnParametersSet() { }
    public virtual void OnAfterRender(bool firstTime) { }
    public virtual void Tick() { }
}

public class PanelComponent : Panel
{
    public Scene Scene => null;
    protected virtual int BuildHash() => 0;
    protected virtual void OnUpdate() { }
    protected virtual void OnAwake() { }
    protected virtual void OnStart() { }
    protected virtual void OnEnabled() { }
    protected virtual void OnDisabled() { }
    protected virtual void OnDestroy() { }
}

public static class HashCode2 { }
