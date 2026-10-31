//! Embedded .NET runtime: runs original C# (Razor `@code` blocks) through the installed `hostfxr`.
//! The managed side (`managed/`) compiles the members with Roslyn into a class, sets the page's values, calls a method and
//! returns the resulting public state and `Log` output as JSON.
use anyhow::{anyhow, bail, Result};
use netcorehost::{hostfxr::Hostfxr, pdcstring::PdCString};
use std::{collections::BTreeMap, path::Path};

type LoadFn = unsafe extern "system" fn(*const u8, i32) -> i32;
type CallFn = unsafe extern "system" fn(i32, *const u8, i32, *const u8, i32, *mut u8, i32) -> i32;
type ErrFn = unsafe extern "system" fn(*mut u8, i32) -> i32;

/// A started .NET runtime with the `SboxHost` assembly loaded.
pub struct Dotnet {
    load: LoadFn,
    call: CallFn,
    err: ErrFn,
}

/// What a call of a page method produced.
#[derive(Debug, Default)]
pub struct CallResult {
    /// Every simple (number/bool/string) field and property after the call, as text.
    pub state: BTreeMap<String, String>,
    /// `Log.Info/Warning/Error` lines the code wrote.
    pub log: Vec<String>,
}

impl Dotnet {
    /// Starts the runtime from `dotnet_root` (the directory holding `host/fxr`) and loads `SboxHost.dll` from `managed_dir`.
    /// Only one runtime can be started per process.
    pub fn start(dotnet_root: &Path, managed_dir: &Path) -> Result<Self> {
        let hostfxr = Hostfxr::load_from_path(find_hostfxr(dotnet_root)?).map_err(|e| anyhow!("hostfxr: {e}"))?;
        let cfg = PdCString::from_os_str(managed_dir.join("SboxHost.runtimeconfig.json").as_os_str())?;
        let ctx = hostfxr.initialize_for_runtime_config(&cfg).map_err(|e| anyhow!("runtime config: {e}"))?;
        let loader = ctx
            .get_delegate_loader_for_assembly(PdCString::from_os_str(managed_dir.join("SboxHost.dll").as_os_str())?)
            .map_err(|e| anyhow!("loader: {e}"))?;
        let ty = PdCString::from_os_str("SboxHost.Host, SboxHost")?;
        let get = |name: &str| -> Result<usize> {
            loader
                .get_function_with_unmanaged_callers_only::<fn()>(&ty, &PdCString::from_os_str(name)?)
                .map(|f| *f as usize)
                .map_err(|e| anyhow!("{name}: {e}"))
        };
        // SAFETY: the managed signatures are declared to match these in Host.cs.
        unsafe {
            Ok(Self {
                load: std::mem::transmute::<usize, LoadFn>(get("Load")?),
                call: std::mem::transmute::<usize, CallFn>(get("Call")?),
                err: std::mem::transmute::<usize, ErrFn>(get("LastError")?),
            })
        }
    }

    fn last_error(&self) -> String {
        let mut buf = vec![0u8; 16384];
        let n = unsafe { (self.err)(buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf8_lossy(&buf[..n.clamp(0, 16384) as usize]).into_owned()
    }

    /// Compiles the members of a `@code` block; returns the page id.
    pub fn load(&self, code: &str) -> Result<i32> {
        let id = unsafe { (self.load)(code.as_ptr(), code.len() as i32) };
        if id < 0 {
            bail!("C# compile failed:\n{}", self.last_error());
        }
        Ok(id)
    }

    /// Sets `values`, calls `method` (empty for none) and returns the page's state afterwards.
    pub fn call(&self, page: i32, method: &str, values: &BTreeMap<String, String>) -> Result<CallResult> {
        let json = format!(
            "{{{}}}",
            values.iter().map(|(k, v)| format!("{}:{}", quote(k), quote(v))).collect::<Vec<_>>().join(",")
        );
        let mut buf = vec![0u8; 1 << 16];
        let n = unsafe {
            (self.call)(page, method.as_ptr(), method.len() as i32, json.as_ptr(), json.len() as i32, buf.as_mut_ptr(), buf.len() as i32)
        };
        if n < 0 {
            bail!("C# call `{method}` failed:\n{}", self.last_error());
        }
        parse_result(std::str::from_utf8(&buf[..n as usize])?)
    }
}

fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Reads `{"state":{k:v,..},"log":[..]}` — a tiny JSON subset reader, enough for what the host emits.
fn parse_result(s: &str) -> Result<CallResult> {
    let mut p = Json { b: s.as_bytes(), i: 0 };
    let mut r = CallResult::default();
    p.eat(b'{')?;
    loop {
        let key = p.string()?;
        p.eat(b':')?;
        if key == "state" {
            p.eat(b'{')?;
            while p.peek() != b'}' {
                let k = p.string()?;
                p.eat(b':')?;
                r.state.insert(k, p.scalar()?);
                p.comma();
            }
            p.eat(b'}')?;
        } else {
            p.eat(b'[')?;
            while p.peek() != b']' {
                r.log.push(p.scalar()?);
                p.comma();
            }
            p.eat(b']')?;
        }
        if !p.comma() {
            break;
        }
    }
    Ok(r)
}

struct Json<'a> {
    b: &'a [u8],
    i: usize,
}

impl Json<'_> {
    fn peek(&self) -> u8 {
        *self.b.get(self.i).unwrap_or(&0)
    }
    fn comma(&mut self) -> bool {
        let c = self.peek() == b',';
        self.i += c as usize;
        c
    }
    fn eat(&mut self, c: u8) -> Result<()> {
        if self.peek() != c {
            bail!("bad json at {}: expected {}", self.i, c as char);
        }
        self.i += 1;
        Ok(())
    }
    fn string(&mut self) -> Result<String> {
        self.eat(b'"')?;
        let mut out = Vec::new();
        while self.peek() != b'"' {
            if self.i >= self.b.len() {
                bail!("unterminated json string");
            }
            if self.peek() == b'\\' {
                self.i += 1;
                match self.peek() {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'u' => {
                        let h = std::str::from_utf8(&self.b[self.i + 1..self.i + 5])?;
                        let mut buf = [0u8; 4];
                        let c = char::from_u32(u32::from_str_radix(h, 16)?).unwrap_or('?');
                        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        self.i += 4;
                    }
                    c => out.push(c),
                }
            } else {
                out.push(self.peek());
            }
            self.i += 1;
        }
        self.i += 1;
        Ok(String::from_utf8(out)?)
    }
    /// A string, number, bool or null, as text.
    fn scalar(&mut self) -> Result<String> {
        if self.peek() == b'"' {
            return self.string();
        }
        let st = self.i;
        while !matches!(self.peek(), b',' | b'}' | b']' | 0) {
            self.i += 1;
        }
        Ok(std::str::from_utf8(&self.b[st..self.i])?.to_string())
    }
}

/// The `hostfxr` library of the newest installed host under `<root>/host/fxr`.
fn find_hostfxr(dotnet_root: &Path) -> Result<std::path::PathBuf> {
    let name = if cfg!(windows) { "hostfxr.dll" } else { "libhostfxr.so" };
    let mut dirs: Vec<_> = std::fs::read_dir(dotnet_root.join("host/fxr"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join(name).exists())
        .collect();
    dirs.sort_by_key(|p| {
        let v: Vec<u32> = p.file_name().unwrap().to_string_lossy().split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse().ok()).collect();
        v
    });
    dirs.pop().map(|d| d.join(name)).ok_or_else(|| anyhow!("no hostfxr under {}", dotnet_root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_razor_code() {
        let root = Path::new("C:/Program Files/dotnet");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("managed/bin");
        if !root.exists() || !dir.join("SboxHost.dll").exists() {
            eprintln!("skipped: no .NET runtime or built SboxHost");
            return;
        }
        let d = Dotnet::start(root, &dir).unwrap();
        let id = d
            .load(
                r#"
                public int Count { get; set; } = 1;
                string Name = "x";
                public string Label => $"{Name}: {Count * 2}";
                public int Total => Enumerable.Range(1, Count).Sum();
                void Bump() { Count += 3; Log.Info("bumped " + Count); }
                "#,
            )
            .unwrap();
        let mut v = BTreeMap::new();
        v.insert("Name".to_string(), "hi \"q\"".to_string());
        let r = d.call(id, "Bump", &v).unwrap();
        assert_eq!(r.state["Count"], "4");
        assert_eq!(r.state["Label"], "hi \"q\": 8");
        assert_eq!(r.state["Total"], "10");
        assert_eq!(r.log, ["info: bumped 4"]);
        // State persists in the instance; a compile error is reported with the diagnostic.
        assert_eq!(d.call(id, "", &BTreeMap::new()).unwrap().state["Count"], "4");
        let e = d.load("void Broken( {").unwrap_err().to_string();
        assert!(e.contains("error CS"), "{e}");
        let e = d.call(id, "Nope", &BTreeMap::new()).unwrap_err().to_string();
        assert!(e.contains("Nope") || e.contains("Null"), "{e}");
    }
}
