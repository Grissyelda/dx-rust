//! How the renderer is created: which graphics API, and wgpu's indirect-draw validation.
//!
//! Every draw goes through indirect multi-draw calls (Bevy's GPU-driven batching). wgpu checks each indirect call's
//! arguments on the GPU (a compute pass per render pass, ~20 buffers of 10 MB kept for it, and the extra tracking
//! when the command buffers are finished and submitted on the render thread). Bevy turns that check off - except
//! when DirectX 12 is among the APIs that may be picked, because wgpu's DX12 backend needs it to pass the first
//! instance of each draw. The default API list includes DX12 on every platform's build, so the check stayed on
//! with Vulkan (Windows, Linux, Steam Deck, Android) and Metal (macOS) too.
//!
//! So the API is named outright where it's known to work: Vulkan on Linux and Android, Metal on macOS, and on Windows
//! Vulkan when a real (non-software) Vulkan GPU answers a quick probe - the same adapter the default list picked
//! on such machines (Vulkan is tried before DX12). Without one, Windows keeps the default list (DX12 then, with the
//! check). `WGPU_BACKEND` set by the player overrides all of this; `DXVIEW_INDIRECT_VALIDATION=1` keeps the check.

use bevy::render::settings::{Backends, InstanceFlags, WgpuSettings};

pub fn wgpu_settings() -> WgpuSettings {
    let mut s = WgpuSettings::default();
    if std::env::var_os("WGPU_BACKEND").is_some() || env_on!("DXVIEW_INDIRECT_VALIDATION") {
        return s;
    }
    let Some(b) = native_backend() else { return s };
    s.backends = Some(b);
    s.instance_flags.remove(InstanceFlags::VALIDATION_INDIRECT_CALL);
    // The player's WGPU_* flag variables still win.
    s.instance_flags = s.instance_flags.with_env();
    println!("gpu: {b:?} only, indirect draw validation off");
    s
}

#[cfg(target_os = "macos")]
fn native_backend() -> Option<Backends> {
    Some(Backends::METAL)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn native_backend() -> Option<Backends> {
    Some(Backends::VULKAN)
}

#[cfg(windows)]
fn native_backend() -> Option<Backends> {
    let t0 = std::time::Instant::now();
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    desc.flags = wgpu::InstanceFlags::empty();
    let inst = wgpu::Instance::new(desc);
    let adapters = bevy::tasks::block_on(inst.enumerate_adapters(wgpu::Backends::VULKAN));
    let real = adapters.iter().any(|a| a.get_info().device_type != wgpu::DeviceType::Cpu);
    println!("gpu: Vulkan probe {} ({} adapters, {} ms)", if real { "ok" } else { "found no GPU" }, adapters.len(), t0.elapsed().as_millis());
    real.then_some(Backends::VULKAN)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux", target_os = "android")))]
fn native_backend() -> Option<Backends> {
    None
}
