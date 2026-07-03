//! Small OS/compiler-portability helpers. (Port of `platform.h`.)
//!
//! The one helper here is runtime CPU feature detection. CPU-feature detection via cpuid is
//! x86-only; on other ISAs (e.g. arm64) there is no AVX-VNNI and `has_avx_vnni()` is a constant
//! false — it is only ever consulted inside the x86 SIMD kernel selection, so it is never
//! actually called off x86.

/// Whether the running CPU supports AVX-VNNI (CPUID.(EAX=7,ECX=1):EAX bit 4).
/// Used to pick the int8 output-dot kernel at runtime. Always false on non-x86 architectures.
#[cfg(target_arch = "x86_64")]
pub fn has_avx_vnni() -> bool {
    // CPUID leaf 7 subleaf 1, EAX bit 4. Check the max leaf first, as __get_cpuid_count does.
    unsafe {
        let max_leaf = core::arch::x86_64::__cpuid(0).eax;
        if max_leaf < 7 {
            return false;
        }
        let regs = core::arch::x86_64::__cpuid_count(7, 1);
        (regs.eax & (1u32 << 4)) != 0
    }
}

/// Always false on non-x86 architectures (no AVX-VNNI there).
#[cfg(not(target_arch = "x86_64"))]
pub fn has_avx_vnni() -> bool {
    false
}
