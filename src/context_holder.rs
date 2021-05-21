use std::cell::UnsafeCell;

use crate::Context;

/// The `ContextHolder` is a wrapper type around the context of a listener.
/// Because listeners only ever process events one-at-a-time and don't share their contexts,
/// it should be fine to have a mutable reference to it within the listener.
pub struct ContextHolder<Ctx: Context> {
    ctx: UnsafeCell<&'static mut Ctx>
}
impl<Ctx: Context> ContextHolder<Ctx> {
    pub fn new(ctx: Ctx) -> Self {
        // This "leak" will be dropped when this holder struct is dropped.
        let ctx: &'static mut Ctx = Box::leak(Box::new(ctx));

        ContextHolder { ctx: UnsafeCell::new(ctx) }
    }

    /// # Safety
    /// This is safe as long as there are no active references to the context.
    /// This is ensured by making this function require a mutable reference to the
    /// `ContextHolder`
    pub fn get_mut(&mut self) -> &'static mut Ctx {
        unsafe { &mut *self.ctx.get() }
    }
}

impl<Ctx: Context> Drop for ContextHolder<Ctx> {
    /// # Safety
    /// This is safe as long as no references to the `ContextHolder` (or its inner context)
    /// are alive. This object is only dropped when the listener is dropped, so that *should*
    /// be fine.
    fn drop(&mut self) {
        let ctx_ref = self.get_mut();
        let ctx_ptr = ctx_ref as *mut Ctx;

        unsafe {
            std::ptr::drop_in_place(ctx_ptr)
        }
    }
}
