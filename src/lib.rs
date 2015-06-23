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
//! 	space.with_lock(filename.as_os_str(),
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
//! // Have 1000 threads increment the value in the file, one at a time
//! for i in 0..1000 {
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
#![feature(alloc)]
extern crate alloc;
// This is only safe if you guard the Arc
use alloc::arc::strong_count;

use std::collections::HashMap;
use std::hash::Hash;
use std::convert::Into;
use std::borrow::Borrow;
use std::sync::{Arc,Mutex,MutexGuard,TryLockError};
use std::ops::{Deref,DerefMut};
use std::mem::{transmute,drop};

pub mod lockresult;
use lockresult::*;

pub mod arcmutexguard;
use arcmutexguard::{ArcMutexGuard,arc_mutex_lock};

mod archashkey;
use archashkey::ArcHashKey;

/// An RAII implementation of a "scoped lock" of a a LockSpace value. When this
/// structure is dropped (falls out of scope), the lock will be unlocked, and
/// the reference count to the key will be decreased by 1.
///
/// The actual value can be accessed through this guard via its Deref and
/// DerefMut implementations.
pub struct LockSpaceGuard<'a,K: 'a + Eq + Hash + Clone,V:'a> {
    owner: &'a LockSpace<K,V>,
    entry: Option<LockSpaceEntry<K,V>>,
    guard: Option<ArcMutexGuard<'a,V>>,
}

impl<'a,K: 'a + Eq + Hash + Clone,V:'a> Deref for LockSpaceGuard<'a,K,V> {
	type Target = V;
	fn deref<'b>(&'b self) -> &'b V {
		// This is always Some, because it's initialized as Some, and only drop() turns it into None
		match self.guard {
			Some(ref value) => &value,
			None => unreachable!(), // to be replace with std::intrinsics::unreachable once stable
		}
	}
}

impl<'a,K: 'a + Eq + Hash + Clone,V:'a> DerefMut for LockSpaceGuard<'a,K,V> {
	fn deref_mut<'b>(&'b mut self) -> &'b mut V {
		// This is always Some, because it's initialized as Some, and only drop() turns it into None
		match self.guard {
			Some(ref mut value) => unsafe{transmute::<&mut V,&'b mut V>(value)},
			None => unreachable!(), // to be replace with std::intrinsics::unreachable once stable
		}
	}
}

impl<'a,K: 'a + Eq + Hash + Clone,V:'a> Drop for LockSpaceGuard<'a,K,V> {
    fn drop(&mut self) {
		self.guard=None; // Release inner lock
		let mut map=self.owner.names.lock().unwrap(); // Acquire outer lock
		{
			let entry=self.entry.take().unwrap();
			if self.owner.cleanup==AutoCleanup {
				// Move our reference to inner before releasing the outer lock
				LockSpace::<K,V>::try_remove_internal(&mut*map,entry);
			}
			// else: drop our reference to inner before releasing the outer lock
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

type LockSpaceEntry<K,V> = (ArcHashKey<K>,Arc<Mutex<V>>);

/// A `LockSpace<K,V>` holds many `Mutex<V>`'s, keyed by `K`.
///
/// All accesses to the internal value must go through one of the lock methods.
///
/// See the crate documentation for an example.
///
/// # Key parameters
/// Most of the `LockSpace<K,V>` methods take a `key: &Q where K: Borrow<Q>`, like a
/// `HashMap`. This basically means "anything that's like a `K`." However, keys
/// are stored internally inside an `Arc`. Not all types that normally have `K:
/// Borrow<Q>`, have `Arc<K>: Borrow<Q>`, and Rust currently does not allow one
/// to implement that generically. This module re-implements the special
/// `Borrow` implementations for `String`, `OsString`, `PathBuf`, `Vec`, and
/// `Cow`. If you are using a custom `K` that has a non-standard `K: Borrow<_>`,
/// you will need to `impl Borrow<_> for ArcHashKey<K>`, for example using the
/// macro `ahk_chain_borrow!`. This macro is used internally like so:
///
/// ```ignore
/// ahk_chain_borrow! {
/// 	String: (),
/// 	OsString: (),
/// 	PathBuf: (),
/// 	Vec<T>: ( T, ),
/// }
/// ```
/// It's possible to specify more than one type parameter this way. Lifetime
/// parameters are not supported.

pub struct LockSpace<K: Eq + Hash,V> {
	// IMPORTANT: To avoid deadlocks, always acquire the inner lock while
	// holding the outer lock. Once the inner lock is acquired, the outer lock
	// can be released.
	names: Arc<Mutex<HashMap<ArcHashKey<K>,LockSpaceEntry<K,V>>>>,
	// IMPORTANT: We implement cleanup based on reference-counting. For this
	// to work, there are a few invariants that must hold:
	//   1. The lock space holds 1 reference to the inner Mutex
	//   2. Each lock guard holds 1 reference to the inner Mutex
	// No. 2 is guaranteed by only cloning it's Arc in two circumstances:
	//   1. When creating a new lock
	//   2. When calling try_remove, emulating a lock
	// For synchronization, the number of references to an inner Mutex is only
	// changed or evaluated while the outer Mutex is locked.
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
impl<K: Eq + Hash + Clone,V> Clone for LockSpace<K,V> {
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
	/// The key may be any borrowed form of the map's key type, but see the struct
	/// documentation for a note.
	///
	/// ```
	/// let space=namedlock::LockSpace::<String,i32>::new(namedlock::KeepUnused);
	///
	/// let value=space.lock("test",||0);
	/// *value.unwrap()+=1;
	/// let value=space.lock("test",||0);
	/// assert_eq!(*value.unwrap(),1);
	pub fn lock<'a,C,Q: ?Sized + Hash + Eq>(&'a self, key: &Q, initial: C) -> LockResult<LockSpaceGuard<'a,K,V>>
		where ArcHashKey<K>: Borrow<Q>, for<'q> &'q Q: Into<K>, /* Take e.g. both &str and &String */
		C: FnOnce() -> V
	{
		let mut map=self.names.lock().unwrap(); // Acquire outer lock

		if !map.contains_key(key.borrow()) {
			// Initialize entry if it does not exist
			let mutex=Arc::new(Mutex::new(initial()));
			let key=ArcHashKey(Arc::new(key.into()));
			map.insert(key.clone(),(key,mutex));
		}

		let target=map.get(key.borrow()).unwrap().clone();
		let result=arc_mutex_lock(target.1.clone(/*Invariants OK*/)); // Acquire inner lock, moving our reference
		drop::<MutexGuard<_>>(map); // Explicitly release outer lock

		result.map(|guard|LockSpaceGuard{owner:self,entry:Some(target),guard:Some(guard)}).map_err(|_|PoisonError::new())
	}

	/// Find the object by `key`, or create it by calling `initial` if it does
	/// not exist. Then, call `f` on that object.
	///
	/// The key may be any borrowed form of the map's key type, but see the struct
	/// documentation for a note.
	///
	/// ```
	/// let space=namedlock::LockSpace::<String,i32>::new(namedlock::KeepUnused);
	///
	/// space.with_lock("test",||0,|i|*i+=1);
	/// assert_eq!(space.with_lock("test",||0,|i|*i).unwrap(),1);
	pub fn with_lock<F,R,C,Q: ?Sized + Hash + Eq>(&self, key: &Q, initial: C, f: F) -> LockResult<R>
		where ArcHashKey<K>: Borrow<Q>, for<'q> &'q Q: Into<K>, /* Take e.g. both &str and &String */
		C: FnOnce() -> V, F: FnOnce(&mut V) -> R
	{
		self.lock(key,initial).map(|mut guard|f(&mut guard)).map_err(|_|PoisonError::new())
	}

	// IMPORTANT: The caller must hold the outer lock
	// to guard target--and therefore map--against data races
	fn try_remove_internal(map: &mut HashMap<ArcHashKey<K>,LockSpaceEntry<K,V>>, target: LockSpaceEntry<K,V>) -> LockSpaceRemoveResult
	{
		// This is the "last" reference if strong_count is 2:
		// - map holds 1 reference (Invariant 1)
		// - target holds 1 reference (Invariant 2)
		if strong_count(&target.1)>2 {
			// This means "a remove() function would block", not "calling lock
			// would block".
			return LockSpaceRemoveResult::WouldBlock
		}

		// If we hold the last reference, delete this entry
		match target.1.try_lock() { // Acquire inner lock
			Ok(_) => match map.remove(&target.0) {
				Some(_) => LockSpaceRemoveResult::Success,
				None => LockSpaceRemoveResult::NotFound,
			},
			Err(TryLockError::WouldBlock) => LockSpaceRemoveResult::WouldBlock,
			Err(TryLockError::Poisoned(_)) => LockSpaceRemoveResult::PoisonError,
		} // Release inner lock
	}

	/// Find the object by `key`, then delete it if it is not actively being
	/// used. If it is actually being used, `WouldBlock` will be returned.
	///
	/// This is only useful if this `LockSpace` is of the `KeepUnused` kind.
	///
	/// The key may be any borrowed form of the map's key type, but see the struct
	/// documentation for a note.
	pub fn try_remove<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> LockSpaceRemoveResult
		where ArcHashKey<K>: Borrow<Q>, /* Take e.g. both &str and &String */
	{
		let mut map=self.names.lock().unwrap(); // Acquire outer lock
		let target;
		if let Some(entry)=map.get(key.borrow()) {
			target=entry.clone(/*Invariants OK*/);
		} else {
			return LockSpaceRemoveResult::NotFound
		}
		Self::try_remove_internal(&mut*map,target) // Move our reference to inner before releasing the outer lock
		// Release outer lock
	}
}

#[cfg(test)]
mod tests {
	use std::thread;
	use std::sync::Arc;
	use super::*;

	#[test]
	#[should_panic(expected="Intializer must run")]
	// A non-deterministic test is better than no test
	fn auto_cleanup() {
		let space=Arc::new(LockSpace::<String,bool>::new(AutoCleanup));
		let mut threads=vec![];

		for _ in 0..1000 {
			let space_clone=space.clone();
			threads.push(thread::spawn(move||{space_clone.with_lock("test",||false,|b|*b=true).unwrap();}));
			let space_clone=space.clone();
			threads.push(thread::spawn(move||{*space_clone.lock("test",||false).unwrap()=true;}));
		}

		for t in threads.into_iter() {
			t.join().unwrap();
		}

		// This should assert since all threads have exited and the automatic
		// cleanup should have run, which means a fresh value should be
		// generated by the initializer
		space.with_lock("test",||panic!("Intializer must run"),|_|{}).unwrap();
	}

	use std::env;
	use std::fs::{OpenOptions,File};
	use std::path::PathBuf;
	use std::ffi::OsString;
	use std::io::{Read,Seek,Write,SeekFrom};
	use std::str::FromStr;

	// Short-hand function for space.lock that opens the file if necessary
	fn file_lock<'a>(space:&'a LockSpace<OsString,File>,filename:Arc<PathBuf>) -> LockSpaceGuard<'a,OsString,File> {
		space.lock(filename.as_os_str(),||OpenOptions::new().read(true).write(true).open(&*filename).unwrap()).unwrap()
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
		for i in 0..1000 {
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
