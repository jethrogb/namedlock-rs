// namedlock - Namespaces for named locks
// Copyright (C) 2015  Jethro G. Beekman
//
// This program is free software; you can redistribute it and/or
// modify it under the terms of the GNU General Public License
// as published by the Free Software Foundation; either version 2
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, write to the Free Software Foundation,
// Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301, USA.

//! Namespaces for named locks.
//!
//! This is useful when synchronizing access to a named resource, but you only
//! know the name of the resource at runtime.
//!
//! For example, you can use this to synchronize access to the filesystem:
//!
//! ```
//! use std::thread;
//! use std::env;
//! use std::fs::{OpenOptions,File};
//! use std::path::PathBuf;
//! use std::ffi::OsString;
//! use std::io::{Read,Seek,Write,SeekFrom};
//! use std::str::FromStr;
//! use std::sync::Arc;
//! use namedlock::{LockSpace,AutoCleanup};
//!
//! // Short-hand function for space.with_lock that opens the file if necessary
//! fn with_file<R,F>(space:LockSpace<OsString,File>,filename:Arc<PathBuf>,f: F) -> R
//! 	where F: FnOnce(&mut File) -> R
//! {
//! 	space.with_lock(filename.as_os_str().to_owned(),
//! 		||OpenOptions::new().read(true).write(true).open(&*filename).unwrap(),f
//! 	).unwrap()
//! }
//!
//! // Initialize the file
//! let mut filename=env::temp_dir();
//! filename.push("namedlock-test");
//! let filename=Arc::new(filename);
//! File::create(&*filename).unwrap().write_all(b"0").unwrap();
//!
//! let space=LockSpace::<OsString,File>::new(AutoCleanup);
//! let mut threads=vec![];
//!
//! // Have 10 threads increment the value in the file, one at a time
//! for i in 0..10 {
//! 	let space_clone=space.clone();
//! 	let filename_clone=filename.clone();
//! 	threads.push(thread::Builder::new().name(format!("{}",i))
//! 		.spawn(move||with_file(space_clone,filename_clone,|file| {
//! 			let mut buf=String::new();
//! 			file.seek(SeekFrom::Start(0)).unwrap();
//! 			file.read_to_string(&mut buf).unwrap();
//! 			file.seek(SeekFrom::Start(0)).unwrap();
//! 			write!(file,"{}",usize::from_str(&buf).unwrap()+1).unwrap();
//! 		})).unwrap()
//! 	);
//! }
//!
//! // Wait until all threads are done
//! let count=threads.len();
//! for t in threads.into_iter() {
//! 	t.join().unwrap();
//! }
//!
//! // Check the result
//! with_file(space,filename,|file| {
//! 	let mut buf=String::new();
//! 	file.seek(SeekFrom::Start(0)).unwrap();
//! 	file.read_to_string(&mut buf).unwrap();
//! 	assert_eq!(count,usize::from_str(&buf).unwrap());
//! });
//! ```
//!
//! ## License
//! namedlock - Copyright (C) 2015  Jethro G. Beekman
//!
//! This program is free software; you can redistribute it and/or
//! modify it under the terms of the GNU General Public License
//! as published by the Free Software Foundation; either version 2
//! of the License, or (at your option) any later version.
//!
//! This program is distributed in the hope that it will be useful,
//! but WITHOUT ANY WARRANTY; without even the implied warranty of
//! MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
//! GNU General Public License for more details.
//!
//! You should have received a copy of the GNU General Public License
//! along with this program; if not, write to the Free Software Foundation,
//! Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301, USA.

#![doc(html_root_url="https://jethrogb.github.io/namedlock-rs/doc/namedlock")]
#![cfg_attr(not(feature="std"),no_std)]
#![cfg_attr(not(feature="std"),feature(alloc))]

#[cfg(all(test,not(feature="std")))] #[macro_use] extern crate std;

#[cfg(feature="spin")] extern crate spin;
#[cfg(feature="std")] extern crate core;
#[cfg(not(feature="std"))] extern crate alloc;
#[cfg(not(feature="std"))] extern crate core_collections;

#[cfg(feature="std")] use std::collections::{hash_map,HashMap};
#[cfg(not(feature="std"))] use core_collections::{hash_map,HashMap};
#[cfg(feature="std")] use std::sync::Arc;
#[cfg(not(feature="std"))] use alloc::arc::Arc;
#[cfg(all(feature="std",not(feature="spin")))] use std::sync::{Mutex,MutexGuard};
#[cfg(feature="spin")] use spin::{Mutex,MutexGuard};
use core::hash::Hash;
use core::ops::{Deref,DerefMut};
use core::mem::drop;

pub mod lockresult;
use lockresult::LockResult as Result;

pub mod ownedmutexguard;
use ownedmutexguard::{OwnedMutex,OwnedMutexGuard};

mod private {
	#[allow(unused_imports)]
	use lockresult::{PoisonError,LockResult};

	pub trait IntoResult<T> {
		fn into_result(self) -> LockResult<T>;
	}

	#[cfg(all(feature="std",not(feature="spin")))]
	impl<T> IntoResult<T> for Result<T,::std::sync::PoisonError<T>> {
		fn into_result(self) -> LockResult<T> {
			match self {
				Ok(v) => Ok(v),
				Err(_) => Err(PoisonError),
			}
		}
	}

	#[cfg(feature="spin")]
	impl<'a,T> IntoResult<::spin::MutexGuard<'a,T>> for ::spin::MutexGuard<'a,T> {
		fn into_result(self) -> LockResult<::spin::MutexGuard<'a,T>> {
			Ok(self)
		}
	}
}
use private::IntoResult;

/// An RAII implementation of a "scoped lock" of a a LockSpace value. When this
/// structure is dropped (falls out of scope), the lock will be unlocked, and
/// the reference count to the key will be decreased by 1.
///
/// The actual value can be accessed through this guard via its Deref and
/// DerefMut implementations.
pub struct LockSpaceGuard<'a,K: 'a + Eq + Hash + Clone,V:'a> {
    owner: &'a LockSpace<K,V>,
    key: Option<K>,
    guard: Option<OwnedMutexGuard<'a,V,Arc<Mutex<V>>>>,
}

impl<'a,K: Eq + Hash + Clone,V:'a> Deref for LockSpaceGuard<'a,K,V> {
	type Target = V;
	fn deref<'b>(&'b self) -> &'b V {
		// This is always Some, because it's initialized as Some, and only drop() turns it into None
		match self.guard {
			Some(ref value) => &value,
			None => unreachable!(), // to be replace with std::intrinsics::unreachable once stable
		}
	}
}

impl<'a,K: Eq + Hash + Clone,V:'a> DerefMut for LockSpaceGuard<'a,K,V> {
	fn deref_mut<'b>(&'b mut self) -> &'b mut V {
		// This is always Some, because it's initialized as Some, and only drop() turns it into None
		match self.guard {
			Some(ref mut value) => unsafe{&mut*(value as *mut _) as &'b mut V},
			None => unreachable!(), // to be replace with std::intrinsics::unreachable once stable
		}
	}
}

impl<'a,K: Eq + Hash + Clone,V:'a> Drop for LockSpaceGuard<'a,K,V> {
    fn drop(&mut self) {
		// release inner lock
		let arc=self.guard.take().unwrap().into_inner();
		// Ignore poison error on drop here
		if let Ok(mut map)=self.owner.names.lock().into_result() { // Acquire outer lock
			// Drop our reference to inner while holding the outer lock. This
			// might drop the Arc reference count to 1, which will later allow
			// Arc::try_unwrap to succeed.
			drop(arc);
			if self.owner.cleanup==AutoCleanup {
				// The following should always match if invariants hold
				if let hash_map::Entry::Occupied(oentry)=map.entry(self.key.take().unwrap()) {
					LockSpace::<K,V>::try_remove_internal(oentry);
				}
			}
		}
		// Release outer lock
    }
}

#[derive(PartialEq,Eq,Clone,Copy)]
pub enum Cleanup {
	KeepUnused,
	AutoCleanup,
}
pub use Cleanup::KeepUnused;
pub use Cleanup::AutoCleanup;

type LockSpaceValue<V> = Option<Arc<Mutex<V>>>;
type LockSpaceEntry<'a,K,V> = hash_map::OccupiedEntry<'a,K,LockSpaceValue<V>>;

/// A `LockSpace<K,V>` holds many `Mutex<V>`'s, keyed by `K`.
///
/// All accesses to the internal value must go through one of the lock methods.
///
/// See the crate documentation for an example.
///
/// # Key parameters
/// Most of the `LockSpace<K,V>` methods take a `key: K`. This is because we
/// make a lot of use of the `HashMap::entry` API. If that API changes to accept
/// e.g. Cow, this crate will adopt that too.
pub struct LockSpace<K: Eq + Hash,V> {
	// IMPORTANT: To avoid deadlocks, always acquire the inner lock while
	// holding the outer lock. Once the inner lock is acquired, the outer lock
	// can be released.
	//
	// Also, when the outer lock is not held, all values must be Some()
	names: Arc<Mutex<HashMap<K,LockSpaceValue<V>>>>,
	// IMPORTANT: We implement cleanup based on reference-counting. For this
	// to work, there are a few invariants that must hold:
	//   1. The lock space holds 1 reference to the inner Mutex
	//   2. Each lock guard holds 1 reference to the inner Mutex
	// No. 2 is guaranteed by only cloning it's Arc in a single circumstance,
	// when creating a new lock. For synchronization, the number of references
	// to an inner Mutex is only changed or evaluated while the outer Mutex is
	// locked.
	cleanup: Cleanup,
}

pub enum LockSpaceRemoveResult {
	Success,
	NotFound,
	PoisonError,
	/// `remove()` would block.
	WouldBlock,
}

// This needs to be implemented manually, since #[derive(Clone)] doesn't
// understand that the type parameters are only used within the Arc<_>
impl<K: Eq + Hash,V> Clone for LockSpace<K,V> {
	fn clone(&self) -> LockSpace<K,V> {
		LockSpace{names:self.names.clone(),cleanup:self.cleanup}
	}
}

impl<K: Eq + Hash + Clone,V> LockSpace<K,V> {
	/// Create a new LockSpace.
	///
	/// If `cleanup` is `AutoCleanup`, values will be deleted automatically when
	/// the last lock is released. Otherwise, values will remain in the space
	/// until `try_remove()` returns `Success`.
	pub fn new(cleanup: Cleanup) -> LockSpace<K,V> {
		LockSpace{names:Arc::new(Mutex::new(HashMap::new())),cleanup:cleanup}
	}

	/// Find the object by `key`, or create it by calling `initial` if it does
	/// not exist. Then, lock it and return a LockSpaceGuard over the object.
	/// Once the guard is dropped, its object is unlocked, and if `AutoCleanup`
	/// is specified for this space, removed if this is the last use.
	///
	/// ```
	/// let space=namedlock::LockSpace::<String,i32>::new(namedlock::KeepUnused);
	///
	/// let value=space.lock("test".to_owned(),||0);
	/// *value.unwrap()+=1;
	/// let value=space.lock("test".to_owned(),||0);
	/// assert_eq!(*value.unwrap(),1);
	pub fn lock<'a,C>(&'a self, key: K, initial: C) -> Result<LockSpaceGuard<'a,K,V>>
		where C: FnOnce() -> V
	{
		let mut map=try!(self.names.lock().into_result()); // Acquire outer lock

		let target={
			map.entry(key.clone())
				.or_insert_with(|| Some(Arc::new(Mutex::new(initial()))))
				.clone(/*Invariants OK*/).unwrap()
		};
		let guard=try!(target.owned_lock()); // Acquire inner lock, moving our reference
		drop::<MutexGuard<_>>(map); // Explicitly release outer lock

		Ok(LockSpaceGuard{owner:self,key:Some(key),guard:Some(guard)})
	}

	/// Find the object by `key`, or create it by calling `initial` if it does
	/// not exist. Then, call `f` on that object.
	///
	/// ```
	/// let space=namedlock::LockSpace::<String,i32>::new(namedlock::KeepUnused);
	///
	/// space.with_lock("test".to_owned(),||0,|i|*i+=1);
	/// assert_eq!(space.with_lock("test".to_owned(),||0,|i|*i).unwrap(),1);
	pub fn with_lock<F,R,C>(&self, key: K, initial: C, f: F) -> Result<R>
		where C: FnOnce() -> V, F: FnOnce(&mut V) -> R
	{
		self.lock(key,initial).map(|mut guard|f(&mut guard))
	}

	// IMPORTANT: The caller must hold the outer lock
	// to guard target--and therefore map--against data races
	fn try_remove_internal<'a>(mut entry: LockSpaceEntry<'a,K,V>) -> LockSpaceRemoveResult
	{
		let arc=entry.get_mut().take().unwrap();
		match Arc::try_unwrap(arc) {
			Ok(_) => {
				entry.remove();
				return LockSpaceRemoveResult::Success
			},
			Err(arc) => {
				*entry.get_mut()=Some(arc);
				return LockSpaceRemoveResult::WouldBlock
			}
		}
	}

	/// Find the object by `key`, then delete it if it is not actively being
	/// used. If it is actually being used, `WouldBlock` will be returned.
	///
	/// This is only useful if this `LockSpace` is of the `KeepUnused` kind.
	pub fn try_remove(&self, key: K) -> LockSpaceRemoveResult
	{
		match self.names.lock().into_result() {
			Ok(mut map) => { // Acquired outer lock
				if let hash_map::Entry::Occupied(entry)=map.entry(key) {
					Self::try_remove_internal(entry)
				} else {
					LockSpaceRemoveResult::NotFound
				}
				// Release outer lock
			},
			Err(_) => LockSpaceRemoveResult::PoisonError
		}
	}
}

#[cfg(test)]
mod tests {
	use std::prelude::v1::*;
	use std::thread;
	use std::sync::Arc;
	use super::*;

	#[cfg(feature="spin")]      const TEST_THREADS: usize = 50;
	#[cfg(not(feature="spin"))] const TEST_THREADS: usize = 1000;

	#[test]
	#[should_panic(expected="Intializer must run")]
	// A non-deterministic test is better than no test
	fn auto_cleanup() {
		let space=Arc::new(LockSpace::<String,bool>::new(AutoCleanup));
		let mut threads=vec![];

		for _ in 0..TEST_THREADS {
			let space_clone=space.clone();
			threads.push(thread::spawn(move||{space_clone.with_lock("test".to_string(),||false,|b|*b=true).unwrap();}));
			let space_clone=space.clone();
			threads.push(thread::spawn(move||{*space_clone.lock("test".to_string(),||false).unwrap()=true;}));
		}

		for t in threads.into_iter() {
			t.join().unwrap();
		}

		// This should assert since all threads have exited and the automatic
		// cleanup should have run, which means a fresh value should be
		// generated by the initializer
		space.with_lock("test".to_string(),||panic!("Intializer must run"),|_|{}).unwrap();
	}

	use std::env;
	use std::fs::{OpenOptions,File};
	use std::path::PathBuf;
	use std::ffi::OsString;
	use std::io::{Read,Seek,Write,SeekFrom};
	use std::str::FromStr;

	// Short-hand function for space.lock that opens the file if necessary
	fn file_lock<'a>(space:&'a LockSpace<OsString,File>,filename:Arc<PathBuf>) -> LockSpaceGuard<'a,OsString,File> {
		space.lock(filename.as_os_str().to_owned(),||OpenOptions::new().read(true).write(true).open(&*filename).unwrap()).unwrap()
	}

	#[test]
	fn file_test_with_lock_space_guard() {
		// Initialize the file
		let mut filename=env::temp_dir();
		filename.push("namedlock-test");
		let filename=Arc::new(filename);
		File::create(&*filename).unwrap().write_all(b"0").unwrap();

		let space=LockSpace::<OsString,File>::new(AutoCleanup);
		let mut threads=vec![];

		// Have 1000 threads increment the value in the file, one at a time
		for i in 0..TEST_THREADS {
			let space_clone=space.clone();
			let filename_clone=filename.clone();
			threads.push(thread::Builder::new().name(format!("{}",i))
				.spawn(move||{
					let mut file=file_lock(&space_clone,filename_clone);
					let mut buf=String::new();
					file.seek(SeekFrom::Start(0)).unwrap();
					file.read_to_string(&mut buf).unwrap();
					file.seek(SeekFrom::Start(0)).unwrap();
					write!(file,"{}",usize::from_str(&buf).unwrap()+1).unwrap();
				}).unwrap()
			);
		}

		// Wait until all threads are done
		let count=threads.len();
		for t in threads.into_iter() {
			t.join().unwrap();
		}

		// Check the result
		let mut file=file_lock(&space,filename);
		let mut buf=String::new();
		file.seek(SeekFrom::Start(0)).unwrap();
		file.read_to_string(&mut buf).unwrap();
		assert_eq!(count,usize::from_str(&buf).unwrap());
	}
}
