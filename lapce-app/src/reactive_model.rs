//! Borrowed reactive projections of headless application models.
use floem::reactive::{RwSignal, SignalTrack, SignalUpdate, SignalWith};

/// A borrowed projection of an application model. The adapter chooses the
/// notification signal, so related fields can share an invalidation domain.
/// Mutation and an optional engine update hook finish before views are notified.
pub struct ModelField<M, T> {
    model: RwSignal<M>,
    changed: RwSignal<()>,
    get: fn(&M) -> &T,
    get_mut: fn(&mut M) -> &mut T,
    on_mutate: Option<fn(&mut M)>,
}

impl<M, T> Copy for ModelField<M, T> {}
impl<M, T> std::fmt::Debug for ModelField<M, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ModelField")
    }
}
impl<M, T> Clone for ModelField<M, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M: 'static, T: 'static> ModelField<M, T> {
    pub fn new(
        model: RwSignal<M>,
        changed: RwSignal<()>,
        get: fn(&M) -> &T,
        get_mut: fn(&mut M) -> &mut T,
        on_mutate: Option<fn(&mut M)>,
    ) -> Self {
        Self {
            model,
            changed,
            get,
            get_mut,
            on_mutate,
        }
    }

    pub fn track(&self) {
        self.changed.track();
    }

    pub fn with<O>(&self, f: impl FnOnce(&T) -> O) -> O {
        self.track();
        self.with_untracked(f)
    }

    pub fn with_untracked<O>(&self, f: impl FnOnce(&T) -> O) -> O {
        self.model.with_untracked(|d| f((self.get)(d)))
    }

    pub fn try_with<O>(&self, f: impl FnOnce(Option<&T>) -> O) -> O {
        self.changed.try_track();
        self.try_with_untracked(f)
    }

    pub fn try_with_untracked<O>(&self, f: impl FnOnce(Option<&T>) -> O) -> O {
        self.model.try_with_untracked(|d| f(d.map(self.get)))
    }

    pub fn set(&self, value: T) {
        self.update(|field| *field = value);
    }

    pub fn update(&self, f: impl FnOnce(&mut T)) {
        self.try_update(f);
    }

    pub fn try_update<O>(&self, f: impl FnOnce(&mut T) -> O) -> Option<O> {
        let result = self.model.try_update(|d| {
            let result = f((self.get_mut)(d));
            if let Some(on_mutate) = self.on_mutate {
                on_mutate(d);
            }
            result
        });
        if result.is_some() {
            self.changed.set(());
        }
        result
    }
}

impl<M: 'static, T: Clone + 'static> ModelField<M, T> {
    pub fn get(&self) -> T {
        self.with(Clone::clone)
    }
    pub fn get_untracked(&self) -> T {
        self.with_untracked(Clone::clone)
    }
    pub fn try_get(&self) -> Option<T> {
        self.try_with(|v| v.cloned())
    }
    pub fn try_get_untracked(&self) -> Option<T> {
        self.try_with_untracked(|v| v.cloned())
    }
}
