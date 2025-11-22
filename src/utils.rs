use std::{mem::MaybeUninit, sync::{Arc, Weak}};

/// Construct a new `Arc<T>` while giving a `Weak<T>` to an allocation function.
/// The allocation function can fail with `E` and the error will be propagated.
/// 
/// Source: https://github.com/multiversx/rc-new-cyclic-fallible/blob/main/src/rc_new_cyclic_fallible.rs
/// Author: (c) Andrei Marinica multiversx
/// Licensed under GPL-3.0
pub fn try_arc_new_cyclic<T, E, F>(f: F) -> Result<Arc<T>, E>
where
    F: FnOnce(&Weak<T>) -> Result<T, E>,
{
    let mut result: Result<(), E> = Ok(());

    let maybe_uninit_arc = Arc::<MaybeUninit<T>>::new_cyclic(|weak_uninit| unsafe {
        // transmute guaranteed to be ok, because MaybeUninit has repr(transparent),
        // additionally, the reference is not going to be used in case of error
        let weak: &Weak<T> = core::mem::transmute(weak_uninit);

        match f(weak) {
            Ok(t) => MaybeUninit::<T>::new(t),
            Err(err) => {
                result = Err(err);
                MaybeUninit::<T>::uninit()
            }
        }
    });
    result?;

    // transmute guaranteed to be ok, because MaybeUninit has repr(transparent)
    let converted: Arc<T> = unsafe { core::mem::transmute(maybe_uninit_arc) };

    Ok(converted)
}