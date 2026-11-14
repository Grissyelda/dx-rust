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

/// The binding-array features Bevy's bindless materials need.
const BINDLESS_FEATURES: bevy::render::settings::WgpuFeatures = bevy::render::settings::WgpuFeatures::TEXTURE_BINDING_ARRAY
    .union(bevy::render::settings::WgpuFeatures::BUFFER_BINDING_ARRAY)
    .union(bevy::render::settings::WgpuFeatures::STORAGE_RESOURCE_BINDING_ARRAY)
    .union(bevy::render::settings::WgpuFeatures::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING)
    .union(bevy::render::settings::WgpuFeatures::PARTIALLY_BOUND_BINDING_ARRAY);


pub fn wgpu_settings() -> WgpuSettings {
    let mut s = WgpuSettings::default();
    // Test aid: no binding arrays, so every material takes its non-bindless path (what phones without them get).
    if env_on!("DXVIEW_NO_BINDLESS") || !bindless_fits() {
        s.disabled_features = Some(BINDLESS_FEATURES);
        println!("gpu: binding arrays off");
    }
    // Test aid: the WebGPU baseline limits (16 sampled textures / samplers per stage...), what small GPUs offer.
    if env_on!("DXVIEW_BASE_LIMITS") {
        s.constrained_limits = Some(bevy::render::settings::WgpuLimits::default());
        println!("gpu: baseline limits (DXVIEW_BASE_LIMITS)");
    }
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

/// Android: may Bevy's bindless materials be used? (Not at present: see below.) Also prints what the device offers
/// (for bug reports). Elsewhere assumed (desktop drivers allow far more).
#[cfg(target_os = "android")]
fn bindless_fits() -> bool {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    desc.flags = wgpu::InstanceFlags::empty();
    let inst = wgpu::Instance::new(desc);
    let adapters = bevy::tasks::block_on(inst.enumerate_adapters(wgpu::Backends::VULKAN));
    let Some(a) = adapters.iter().find(|a| a.get_info().device_type != wgpu::DeviceType::Cpu).or(adapters.first()) else {
        return true;
    };
    let (info, l, f) = (a.get_info(), a.limits(), a.features());
    let has = f.contains(BINDLESS_FEATURES);
    // Bindless materials stay off on Android for now: the phones that run the game (Mali, no binding arrays) take
    // the plain path, and the ones that drop to the background at start (GitHub issue #62: Snapdragon / Adreno,
    // which offer binding arrays) are the ones that would take the bindless one - so all take the plain path.
    let fits = !has;
    let line = format!(
        "gpu: {} ({}) binding arrays {} / {} elements, storage textures {}, storage buffers {}, compute x {}, 2D {}",
        info.name, info.driver_info, has, l.max_binding_array_elements_per_shader_stage,
        l.max_storage_textures_per_shader_stage, l.max_storage_buffers_per_shader_stage,
        l.max_compute_workgroup_size_x, l.max_texture_dimension_2d
    );
    println!("{line}");
    crate::platform::note(&line);
    fits
}

#[cfg(not(target_os = "android"))]
fn bindless_fits() -> bool {
    true
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
