//! The process-wide embedded .NET runtime (`--dotnet`): a Razor page's `@code` block runs as the original C# when the
//! small built-in subset in `csharp.rs` is not enough.
use std::path::PathBuf;
use std::sync::OnceLock;

use sbox_dotnet::Dotnet;

static RUNTIME: OnceLock<Option<Dotnet>> = OnceLock::new();

/// Starts the runtime once. `SBOX_DOTNET_ROOT` / `SBOX_MANAGED_DIR` override where .NET and the built `SboxHost.dll` are.
pub fn enable() {
    RUNTIME.get_or_init(|| {
        let root = std::env::var_os("SBOX_DOTNET_ROOT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:/Program Files/dotnet"));
        let dir = std::env::var_os("SBOX_MANAGED_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../sbox-dotnet/managed/bin")));
        match Dotnet::start(&root, &dir) {
            Ok(d) => Some(d),
            Err(e) => {
                log::error!("--dotnet: {e:#}");
                None
            }
        }
    });
}

/// The runtime, when `--dotnet` started it.
pub fn get() -> Option<&'static Dotnet> {
    RUNTIME.get().and_then(|d| d.as_ref())
}
