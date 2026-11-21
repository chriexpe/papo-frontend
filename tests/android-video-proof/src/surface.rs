//! Owns one strong ANativeWindow reference and writes only checked, current native frames.

use papo_call::PackedBgra;

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PostOutcome {
    Posted,
    DiscardedStale,
    Failed,
}

pub(crate) trait PresentWindow: Send {
    fn present_rgba(
        &mut self,
        bgra: &PackedBgra,
        still_current: &dyn Fn() -> bool,
        stall_hook: &dyn Fn(),
    ) -> PostOutcome;
}

#[cfg(target_os = "android")]
mod android {
    use super::{PostOutcome, PresentWindow};
    use jni::{JNIEnv, objects::JObject};
    use ndk_sys as ndk;
    use papo_call::PackedBgra;
    use std::{
        mem::MaybeUninit,
        ptr::{NonNull, null_mut},
    };

    enum WriteOutcome {
        Written,
        Stale,
        LayoutError,
    }

    // ANativeWindow_setBuffersGeometry accepts AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM.
    const RGBA_8888: i32 = 1;

    pub struct Window {
        raw: NonNull<ndk::ANativeWindow>,
        size_px: [u32; 2],
    }

    // SAFETY: Android's native window reference can move threads; &mut self serializes writes.
    unsafe impl Send for Window {}

    impl Window {
        pub fn from_surface(env: &JNIEnv<'_>, surface: &JObject<'_>) -> Option<Self> {
            if surface.is_null() {
                return None;
            }
            // SAFETY: Env and Surface are live JNI references on the calling thread.
            let raw = unsafe {
                ndk::ANativeWindow_fromSurface(env.get_raw().cast(), surface.as_raw().cast())
            };
            NonNull::new(raw).map(|raw| Self {
                raw,
                size_px: [0, 0],
            })
        }
    }

    impl PresentWindow for Window {
        fn present_rgba(
            &mut self,
            bgra: &PackedBgra,
            still_current: &dyn Fn() -> bool,
            stall_hook: &dyn Fn(),
        ) -> PostOutcome {
            if !still_current() {
                return PostOutcome::DiscardedStale;
            }
            let [width_px, height_px] = bgra.size_px;
            let (Ok(width), Ok(height)) = (i32::try_from(width_px), i32::try_from(height_px))
            else {
                return PostOutcome::Failed;
            };
            let window = self.raw.as_ptr();
            if self.size_px != bgra.size_px {
                // SAFETY: The owning reference stays alive and this producer owns the window.
                let rc = unsafe {
                    ndk::ANativeWindow_setBuffersGeometry(window, width, height, RGBA_8888)
                };
                if rc != 0 {
                    return PostOutcome::Failed;
                }
                self.size_px = bgra.size_px;
            }
            let mut buffer = MaybeUninit::uninit();
            // SAFETY: Android initializes buffer on success. No dirty region limits our write.
            if unsafe { ndk::ANativeWindow_lock(window, buffer.as_mut_ptr(), null_mut()) } != 0 {
                return PostOutcome::Failed;
            }
            // Inject any armed proof stall while owning the native lock before writing pixels.
            stall_hook();
            // SAFETY: Successful lock initialized the descriptor; storage lives until unlockAndPost.
            let buffer = unsafe { buffer.assume_init() };
            let write_outcome = write_rgba(&buffer, bgra, still_current);
            // SAFETY: Each successful lock must be unlocked exactly once, even for stale frames.
            let posted = unsafe { ndk::ANativeWindow_unlockAndPost(window) } == 0;
            if !posted {
                return PostOutcome::Failed;
            }
            match write_outcome {
                WriteOutcome::Written => PostOutcome::Posted,
                WriteOutcome::Stale => PostOutcome::DiscardedStale,
                WriteOutcome::LayoutError => PostOutcome::Failed,
            }
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: Balances the reference acquired by ANativeWindow_fromSurface.
            unsafe { ndk::ANativeWindow_release(self.raw.as_ptr()) };
        }
    }

    fn write_rgba(
        buffer: &ndk::ANativeWindow_Buffer,
        bgra: &PackedBgra,
        still_current: &dyn Fn() -> bool,
    ) -> WriteOutcome {
        let (Ok(width), Ok(height), Ok(stride)) = (
            usize::try_from(buffer.width),
            usize::try_from(buffer.height),
            usize::try_from(buffer.stride),
        ) else {
            return WriteOutcome::LayoutError;
        };
        if buffer.format != RGBA_8888
            || buffer.bits.is_null()
            || width == 0
            || height == 0
            || stride < width
        {
            return WriteOutcome::LayoutError;
        }
        let Some(stride_bytes) = stride.checked_mul(4) else {
            return WriteOutcome::LayoutError;
        };
        let Some(size_bytes) = stride_bytes
            .checked_mul(height)
            .filter(|size| *size <= isize::MAX as usize)
        else {
            return WriteOutcome::LayoutError;
        };
        // SAFETY: The buffer is locked and RGBA8888 gives four bytes per pixel, including stride.
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(buffer.bits.cast::<MaybeUninit<u8>>(), size_bytes)
        };
        let row_bytes = width * 4;
        let matching = buffer.width as u32 == bgra.size_px[0]
            && buffer.height as u32 == bgra.size_px[1]
            && bgra.bytes.len() == row_bytes * height;
        if !still_current() {
            // A lost lease must not publish old pixels; zero-fill locked storage before unlockAndPost.
            bytes.fill(MaybeUninit::new(0));
            return WriteOutcome::Stale;
        }
        if !matching {
            bytes.fill(MaybeUninit::new(0));
            return WriteOutcome::LayoutError;
        }
        for (src, dst) in bgra
            .bytes
            .chunks_exact(row_bytes)
            .zip(bytes.chunks_exact_mut(stride_bytes))
        {
            let (src_pixels, _) = src.as_chunks::<4>();
            let (dst_pixels, _) = dst[..row_bytes].as_chunks_mut::<4>();
            for (src, dst) in src_pixels.iter().zip(dst_pixels) {
                for (channel, value) in dst.iter_mut().zip([src[2], src[1], src[0], 255]) {
                    channel.write(value);
                }
            }
        }
        if still_current() {
            WriteOutcome::Written
        } else {
            // A lost lease must not publish old pixels; zero-fill locked storage before unlockAndPost.
            bytes.fill(MaybeUninit::new(0));
            WriteOutcome::Stale
        }
    }
}

#[cfg(target_os = "android")]
pub use android::Window;

#[cfg(not(target_os = "android"))]
pub struct Window;

#[cfg(not(target_os = "android"))]
impl Window {
    pub fn from_surface(
        _env: &jni::JNIEnv<'_>,
        _surface: &jni::objects::JObject<'_>,
    ) -> Option<Self> {
        None
    }
}

#[cfg(not(target_os = "android"))]
impl PresentWindow for Window {
    fn present_rgba(
        &mut self,
        _bgra: &PackedBgra,
        _still_current: &dyn Fn() -> bool,
        _stall_hook: &dyn Fn(),
    ) -> PostOutcome {
        PostOutcome::Failed
    }
}
