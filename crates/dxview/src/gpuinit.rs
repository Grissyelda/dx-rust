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


/// The GPU's name as the probe saw it (Android and Arm Linux; unset elsewhere).
static GPU_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// A phone GPU that takes the simplest drawing path we have (Arm Mali, or `DXVIEW_SAFE_GPU=1` to try it on a desktop):
/// no GPU-driven indirect draws (Bevy's NoIndirectDrawing on every camera) or GPU culling,
/// characters posed on the CPU instead of by the vertex-animation shader, and no save thumbnails (no screen read-back).
/// A Mali phone closed when Training or a new game loaded its first map, while the title screen (no characters, no
/// save) ran; these are the paths that start there.
pub fn safe_gpu() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = env_on!("DXVIEW_SAFE_GPU") || family() == GpuFamily::Mali;
        if on {
            println!("gpu: simple drawing path (no indirect draws or GPU culling, CPU character animation, no save thumbnails)");
            crate::platform::note("simple GPU path on: no indirect draws or GPU culling, CPU character animation, no save thumbnails");
        }
        on
    })
}

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

/// What the probe found (Android and Arm Linux): the GPU's name and whether it offers binding arrays.
#[cfg(any(target_os = "android", all(target_os = "linux", target_arch = "aarch64")))]
static PROBE: std::sync::OnceLock<Option<(String, bool)>> = std::sync::OnceLock::new();

/// Android and Arm Linux: look at the GPU once, before anything decides on the look or the mods (Snapdragon /
/// Adreno phones get Modern and mods, Mali ones the Classic game: `modern_capable`; Mali GPUs on phones and on Arm
/// Linux boards / handhelds (Panfrost, Panthor) take the simple drawing path: `safe_gpu`). Prints what the device
/// offers (for bug reports). Elsewhere nothing to do.
pub fn probe() {
    #[cfg(any(target_os = "android", all(target_os = "linux", target_arch = "aarch64")))]
    PROBE.get_or_init(|| {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::VULKAN;
        desc.flags = wgpu::InstanceFlags::empty();
        let inst = wgpu::Instance::new(desc);
        let adapters = bevy::tasks::block_on(inst.enumerate_adapters(wgpu::Backends::VULKAN));
        let a = adapters.iter().find(|a| a.get_info().device_type != wgpu::DeviceType::Cpu).or(adapters.first())?;
        let (info, l, f) = (a.get_info(), a.limits(), a.features());
        let _ = GPU_NAME.set(info.name.clone());
        let has = f.contains(BINDLESS_FEATURES);
        let line = format!(
            "gpu: {} ({}) binding arrays {} / {} elements, storage textures {}, storage buffers {}, compute x {}, 2D {}",
            info.name, info.driver_info, has, l.max_binding_array_elements_per_shader_stage,
            l.max_storage_textures_per_shader_stage, l.max_storage_buffers_per_shader_stage,
            l.max_compute_workgroup_size_x, l.max_texture_dimension_2d
        );
        println!("{line}");
        crate::platform::note(&line);
        Some((info.name.clone(), has))
    });
}

/// The phone's GPU family (Android); `Desktop` elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuFamily {
    Desktop,
    /// Qualcomm Snapdragon.
    Adreno,
    /// Arm.
    Mali,
    /// PowerVR, Samsung Xclipse and others.
    OtherMobile,
}

/// (Desktop GPUs aren't probed: Desktop.)
pub fn family() -> GpuFamily {
    probe();
    let Some(name) = GPU_NAME.get().map(String::as_str) else { return GpuFamily::Desktop };
    if crate::util::contains_ci(name, "adreno") {
        GpuFamily::Adreno
    } else if crate::util::contains_ci(name, "mali") || crate::util::contains_ci(name, "immortalis") {
        GpuFamily::Mali
    } else {
        GpuFamily::OtherMobile
    }
}

/// Modern, its enhancements and mods may run here: every desktop, and Snapdragon (Adreno) phones. Mali and other
/// phone GPUs play the Classic game without mods. `DXVIEW_ANDROID_MODERN=1` / `=0` decides for a test.
pub fn modern_capable() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        if !crate::platform::android() {
            return true;
        }
        let on = match std::env::var("DXVIEW_ANDROID_MODERN").ok().as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => family() == GpuFamily::Adreno,
        };
        crate::platform::note(if on { "Modern and mods available (Snapdragon)" } else { "Classic only (this GPU)" });
        on
    })
}

/// Android: may Bevy's bindless materials be used? Not at present: the phones that first ran the game (Mali) have
/// no binding arrays, and Snapdragon (Adreno) ones that offer them dropped to the background at start while they
/// took that path (GitHub issue #62), so every phone takes the plain one.
#[cfg(target_os = "android")]
fn bindless_fits() -> bool {
    probe();
    PROBE.get().and_then(|p| p.as_ref()).is_none_or(|(_, has)| !has)
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
