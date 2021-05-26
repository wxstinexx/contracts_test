// Copyright 2018-2020 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use super::{
    CacheCell,
    EntryState,
    StorageEntry,
};
use crate::traits::{
    clear_spread_root_opt,
    pull_spread_root_opt,
    ExtKeyPtr,
    KeyPtr,
    SpreadLayout,
};
use core::{
    fmt,
    fmt::Debug,
    ptr::NonNull,
};
use ink_primitives::Key;

/// A lazy storage entity.
///
/// This loads its value from storage upon first use.
///
/// # Note
///
/// Use this if the storage field doesn't need to be loaded in some or most cases.
pub struct LazyCell<T>
where
    T: SpreadLayout,
{
    /// The key to lazily load the value from.
    ///
    /// # Note
    ///
    /// This can be `None` on contract initialization where a `LazyCell` is
    /// normally initialized given a concrete value.
    key: Option<Key>,
    /// The low-level cache for the lazily loaded storage value.
    ///
    /// # Safety (Dev)
    ///
    /// We use `UnsafeCell` instead of `RefCell` because
    /// the intended use-case is to hand out references (`&` and `&mut`)
    /// to the callers of `Lazy`. This cannot be done without `unsafe`
    /// code even with `RefCell`. Also `RefCell` has a larger memory footprint
    /// and has additional overhead that we can avoid by the interface
    /// and the fact that ink! code is always run single-threaded.
    /// Being efficient is important here because this is intended to be
    /// a low-level primitive with lots of dependencies.
    cache: CacheCell<Option<StorageEntry<T>>>,
}

impl<T> Debug for LazyCell<T>
where
    T: Debug + SpreadLayout,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("LazyCell")
            .field("key", &self.key)
            .field("cache", self.cache.as_inner())
            .finish()
    }
}

#[test]
fn debug_impl_works() {
    let c1 = <LazyCell<i32>>::new(None);
    assert_eq!(
        format!("{:?}", &c1),
        "LazyCell { key: None, cache: Some(Entry { value: None, state: Mutated }) }",
    );
    let c2 = <LazyCell<i32>>::new(Some(42));
    assert_eq!(
        format!("{:?}", &c2),
        "LazyCell { key: None, cache: Some(Entry { value: Some(42), state: Mutated }) }",
    );
    let c3 = <LazyCell<i32>>::lazy(Key::from([0x00; 32]));
    assert_eq!(
        format!("{:?}", &c3),
        "LazyCell { \
            key: Some(Key(0x_\
                0000000000000000_\
                0000000000000000_\
                0000000000000000_\
                0000000000000000)\
            ), \
            cache: None \
        }",
    );
}

impl<T> Drop for LazyCell<T>
where
    T: SpreadLayout,
{
    fn drop(&mut self) {
        if let Some(key) = self.key() {
            if let Some(entry) = self.entry() {
                clear_spread_root_opt::<T, _>(key, || entry.value().into())
            }
        }
    }
}

#[cfg(feature = "std")]
const _: () = {
    use crate::traits::StorageLayout;
    use ink_metadata::layout::Layout;

    impl<T> StorageLayout for LazyCell<T>
    where
        T: StorageLayout + SpreadLayout,
    {
        fn layout(key_ptr: &mut KeyPtr) -> Layout {
            <T as StorageLayout>::layout(key_ptr)
        }
    }
};

impl<T> SpreadLayout for LazyCell<T>
where
    T: SpreadLayout,
{
    const FOOTPRINT: u64 = <T as SpreadLayout>::FOOTPRINT;

    fn pull_spread(ptr: &mut KeyPtr) -> Self {
        Self::lazy(*KeyPtr::next_for::<T>(ptr))
    }

    fn push_spread(&self, ptr: &mut KeyPtr) {
        if let Some(entry) = self.entry() {
            SpreadLayout::push_spread(entry, ptr)
        }
    }

    fn clear_spread(&self, ptr: &mut KeyPtr) {
        if let Some(entry) = self.entry() {
            SpreadLayout::clear_spread(entry, ptr)
        }
    }
}

// # Developer Note
//
// Implementing PackedLayout for LazyCell is not useful since that would
// potentially allow overlapping distinct LazyCell instances by pulling
// from the same underlying storage cell.
//
// If a user wants a packed LazyCell they can instead pack its inner type.

impl<T> From<T> for LazyCell<T>
where
    T: SpreadLayout,
{
    fn from(value: T) -> Self {
        Self::new(Some(value))
    }
}

impl<T> Default for LazyCell<T>
where
    T: Default + SpreadLayout,
{
    fn default() -> Self {
        Self::new(Some(Default::default()))
    }
}

impl<T> LazyCell<T>
where
    T: SpreadLayout,
{
    /// Creates an already populated lazy storage cell.
    ///
    /// # Note
    ///
    /// Since this already has a value it will never actually load from
    /// the contract storage.
    #[must_use]
    pub fn new(value: Option<T>) -> Self {
        Self {
            key: None,
            cache: CacheCell::new(Some(StorageEntry::new(value, EntryState::Mutated))),
        }
    }

    /// Creates a lazy storage cell for the given key.
    ///
    /// # Note
    ///
    /// This will actually lazily load from the associated storage cell
    /// upon access.
    #[must_use]
    pub fn lazy(key: Key) -> Self {
        Self {
            key: Some(key),
            cache: CacheCell::new(None),
        }
    }

    /// Returns the lazy key if any.
    ///
    /// # Note
    ///
    /// The key is `None` if the `LazyCell` has been initialized as a value.
    /// This generally only happens in ink! constructors.
    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// Returns the cached entry.
    fn entry(&self) -> Option<&StorageEntry<T>> {
        self.cache.as_inner().as_ref()
    }
}

impl<T> LazyCell<T>
where
    T: SpreadLayout,
{
    /// Loads the storage entry.
    ///
    /// Tries to load the entry from cache and falls back to lazily load the
    /// entry from the contract storage.
    unsafe fn load_through_cache(&self) -> NonNull<StorageEntry<T>> {
        // SAFETY: This is critical because we mutably access the entry.
        //         However, we mutate the entry only if it is vacant.
        //         If the entry is occupied by a value we return early.
        //         This way we do not invalidate pointers to this value.
        let cache = &mut *self.cache.get_ptr().as_ptr();
        if cache.is_none() {
            // Load value from storage and then return the cached entry.
            let value = self
                .key
                .map(|key| pull_spread_root_opt::<T>(&key))
                .unwrap_or(None);
            *cache = Some(StorageEntry::new(value, EntryState::Preserved));
        }
        debug_assert!(cache.is_some());
        NonNull::from(cache.as_mut().expect("unpopulated cache entry"))
    }

    /// Returns a shared reference to the entry.
    fn load_entry(&self) -> &StorageEntry<T> {
        // SAFETY: We load the entry either from cache of from contract storage.
        //
        //         This is safe because we are just returning a shared reference
        //         from within a `&self` method. This also cannot change the
        //         loaded value and thus cannot change the `mutate` flag of the
        //         entry. Aliases using this method are safe since ink! is
        //         single-threaded.
        unsafe { &*self.load_through_cache().as_ptr() }
    }

    /// Returns an exclusive reference to the entry.
    fn load_entry_mut(&mut self) -> &mut StorageEntry<T> {
        // SAFETY: We load the entry either from cache of from contract storage.
        //
        //         This is safe because we are just returning an exclusive reference
        //         from within a `&mut self` method. This may change the
        //         loaded value and thus the `mutate` flag of the entry is set.
        //         Aliases cannot happen through this method since ink! is
        //         single-threaded.
        let entry = unsafe { &mut *self.load_through_cache().as_ptr() };
        entry.replace_state(EntryState::Mutated);
        entry
    }

    /// Returns a shared reference to the value.
    ///
    /// # Note
    ///
    /// This eventually lazily loads the value from the contract storage.
    ///
    /// # Panics
    ///
    /// If decoding the loaded value to `T` failed.
    #[must_use]
    pub fn get(&self) -> Option<&T> {
        self.load_entry().value().into()
    }

    /// Returns an exclusive reference to the value.
    ///
    /// # Note
    ///
    /// This eventually lazily loads the value from the contract storage.
    ///
    /// # Panics
    ///
    /// If decoding the loaded value to `T` failed.
    #[must_use]
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.load_entry_mut().value_mut().into()
    }

    /// Sets the value in this cell to `value`, without executing any reads.
    ///
    /// # Note
    ///
    /// No reads from contract storage will be executed.
    ///
    /// This method should be preferred over dereferencing or `get_mut`
    /// in case the returned value is of no interest to the caller.
    ///
    /// # Panics
    ///
    /// If accessing the inner value fails.
    #[inline]
    pub fn set(&mut self, new_value: T) {
        // SAFETY: This is critical because we mutably access the entry.
        let cache = unsafe { &mut *self.cache.get_ptr().as_ptr() };
        if let Some(cache) = cache.as_mut() {
            //  Cache is already populated we simply overwrite its already existing value.
            cache.put(Some(new_value));
        } else {
            // Cache is empty, so we simply set the cache to the value.
            // The key does not need to exist for this to work, we only need to
            // write the value into the cache and are done. Writing to contract
            // storage happens during setup/teardown of a contract.
            *cache = Some(StorageEntry::new(Some(new_value), EntryState::Mutated));
        }
        debug_assert!(cache.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EntryState,
        LazyCell,
        StorageEntry,
    };
    use crate::{
        traits::{
            KeyPtr,
            SpreadLayout,
        },
        Lazy,
    };
    use ink_env::test::run_test;
    use ink_primitives::Key;

    #[test]
    fn new_works() {
        // Initialized via some value:
        let mut a = <LazyCell<u8>>::new(Some(b'A'));
        assert_eq!(a.key(), None);
        assert_eq!(
            a.entry(),
            Some(&StorageEntry::new(Some(b'A'), EntryState::Mutated))
        );
        assert_eq!(a.get(), Some(&b'A'));
        assert_eq!(a.get_mut(), Some(&mut b'A'));
        // Initialized as none:
        let mut b = <LazyCell<u8>>::new(None);
        assert_eq!(b.key(), None);
        assert_eq!(
            b.entry(),
            Some(&StorageEntry::new(None, EntryState::Mutated))
        );
        assert_eq!(b.get(), None);
        assert_eq!(b.get_mut(), None);
        // Same as default or from:
        let default_lc = <LazyCell<u8>>::default();
        let from_lc = LazyCell::from(u8::default());
        let new_lc = LazyCell::new(Some(u8::default()));
        assert_eq!(default_lc.get(), from_lc.get());
        assert_eq!(from_lc.get(), new_lc.get());
        assert_eq!(new_lc.get(), Some(&u8::default()));
    }

    #[test]
    fn lazy_works() {
        let root_key = Key::from([0x42; 32]);
        let cell = <LazyCell<u8>>::lazy(root_key);
        assert_eq!(cell.key(), Some(&root_key));
    }

    #[test]
    fn lazy_get_works() -> ink_env::Result<()> {
        run_test::<ink_env::DefaultEnvironment, _>(|_| {
            let cell = <LazyCell<u8>>::lazy(Key::from([0x42; 32]));
            let value = cell.get();
            // We do the normally unreachable check in order to have an easier
            // time finding the issue if the above execution did not panic.
            assert_eq!(value, None);
            Ok(())
        })
    }

    #[test]
    fn get_mut_works() {
        let mut cell = <LazyCell<i32>>::new(Some(1));
        assert_eq!(cell.get(), Some(&1));
        *cell.get_mut().unwrap() += 1;
        assert_eq!(cell.get(), Some(&2));
    }

    #[test]
    fn spread_layout_works() -> ink_env::Result<()> {
        run_test::<ink_env::DefaultEnvironment, _>(|_| {
            let cell_a0 = <LazyCell<u8>>::new(Some(b'A'));
            assert_eq!(cell_a0.get(), Some(&b'A'));
            // Push `cell_a0` to the contract storage.
            // Then, pull `cell_a1` from the contract storage and check if it is
            // equal to `cell_a0`.
            let root_key = Key::from([0x42; 32]);
            SpreadLayout::push_spread(&cell_a0, &mut KeyPtr::from(root_key));
            let cell_a1 =
                <LazyCell<u8> as SpreadLayout>::pull_spread(&mut KeyPtr::from(root_key));
            assert_eq!(cell_a1.get(), cell_a0.get());
            assert_eq!(cell_a1.get(), Some(&b'A'));
            assert_eq!(
                cell_a1.entry(),
                Some(&StorageEntry::new(Some(b'A'), EntryState::Preserved))
            );
            // Also test if a lazily instantiated cell works:
            let cell_a2 = <LazyCell<u8>>::lazy(root_key);
            assert_eq!(cell_a2.get(), cell_a0.get());
            assert_eq!(cell_a2.get(), Some(&b'A'));
            assert_eq!(
                cell_a2.entry(),
                Some(&StorageEntry::new(Some(b'A'), EntryState::Preserved))
            );
            // Test if clearing works:
            SpreadLayout::clear_spread(&cell_a1, &mut KeyPtr::from(root_key));
            let cell_a3 = <LazyCell<u8>>::lazy(root_key);
            assert_eq!(cell_a3.get(), None);
            assert_eq!(
                cell_a3.entry(),
                Some(&StorageEntry::new(None, EntryState::Preserved))
            );
            Ok(())
        })
    }

    #[test]
    fn set_works() {
        let mut cell = <LazyCell<i32>>::new(Some(1));
        cell.set(23);
        assert_eq!(cell.get(), Some(&23));
    }

    #[test]
    fn lazy_set_works() -> ink_env::Result<()> {
        run_test::<ink_env::DefaultEnvironment, _>(|_| {
            let mut cell = <LazyCell<u8>>::lazy(Key::from([0x42; 32]));
            let value = cell.get();
            assert_eq!(value, None);

            cell.set(13);
            assert_eq!(cell.get(), Some(&13));
            Ok(())
        })
    }

    #[test]
    fn lazy_set_works_with_spread_layout_push_pull() -> ink_env::Result<()> {
        run_test::<ink_env::DefaultEnvironment, _>(|_| {
            type MaybeValue = Option<u8>;

            // Initialize a LazyCell with None and push it to `k`
            let k = Key::from([0x00; 32]);
            let val: MaybeValue = None;
            SpreadLayout::push_spread(&Lazy::new(val), &mut KeyPtr::from(k));

            // Pull another instance `v` from `k`, check that it is `None`
            let mut v =
                <Lazy<MaybeValue> as SpreadLayout>::pull_spread(&mut KeyPtr::from(k));
            assert_eq!(*v, None);

            // Set `v` using `set` to an actual value
            let actual_value: MaybeValue = Some(13);
            Lazy::set(&mut v, actual_value);

            // Push `v` to `k`
            SpreadLayout::push_spread(&v, &mut KeyPtr::from(k));

            // Load `v2` from `k`
            let v2 =
                <Lazy<MaybeValue> as SpreadLayout>::pull_spread(&mut KeyPtr::from(k));

            // Check that V2 is the set value
            assert_eq!(*v2, Some(13));

            Ok(())
        })
    }
}
