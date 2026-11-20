//! The upper halves of the vector registers, left in use by code outside
//! the workspace.
//!
//! faer's matrix products on x86-64 run hand-written AVX and AVX-512
//! kernels (the `private-gemm-x86` crate) that return without
//! `vzeroupper`. The upper halves of the vector registers then stay in
//! use, and on Intel processors every later SSE instruction on that thread
//! (Cranelift's code, the interpreter, libm) waits on them: a 200-link
//! chain stepped by RK4 went from 13 ms to 230 ms a step once a 50-link
//! chain had been factorised by TR-BDF2 in the same process
//! (`docs/design/performance.md`, 2026-11-19). Clearing them after faer
//! returns costs a cycle and changes no value.

/// Clear the upper halves of the vector registers (`vzeroupper`), when the
/// processor has AVX. Call it after code that may leave them in use.
pub fn clear_upper() {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx") {
        // SAFETY: the processor has AVX, checked just above.
        #[allow(unsafe_code)]
        unsafe {
            zero_upper();
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
fn zero_upper() {
    std::arch::x86_64::_mm256_zeroupper();
}

/// Whether the upper halves of the vector registers are in use on this
/// thread (`XGETBV` with ECX = 1: the AVX and AVX-512 upper states);
/// `None` where the processor cannot tell. For tests.
pub fn upper_in_use() -> Option<bool> {
    #[cfg(target_arch = "x86_64")]
    {
        let cpuid = std::arch::x86_64::__cpuid_count;
        // XGETBV with ECX = 1 exists when CPUID.(0DH, 1):EAX bit 2 is set,
        // which needs leaf 0DH and OSXSAVE (CPUID.1:ECX bit 27).
        if cpuid(0, 0).eax < 0xd || cpuid(1, 0).ecx & (1 << 27) == 0 || cpuid(0xd, 1).eax & 4 == 0 {
            return None;
        }
        // SAFETY: XGETBV with ECX = 1 is supported, checked just above.
        #[allow(unsafe_code)]
        let in_use = unsafe { xinuse() };
        // Bit 2: the upper halves of YMM0–15; bit 6: of ZMM0–15.
        Some(in_use & (1 << 2 | 1 << 6) != 0)
    }
    #[cfg(not(target_arch = "x86_64"))]
    None
}

/// # Safety
/// The processor must support `XGETBV` with ECX = 1.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
#[target_feature(enable = "xsave")]
unsafe fn xinuse() -> u64 {
    // SAFETY: the caller checked that ECX = 1 is supported.
    unsafe { std::arch::x86_64::_xgetbv(1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_leaves_the_upper_halves_unused() {
        clear_upper();
        if let Some(in_use) = upper_in_use() {
            assert!(!in_use);
        }
    }
}
