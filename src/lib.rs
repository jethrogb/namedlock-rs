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
//! use namedlock::LockSpace;
//!
//! // Short-hand function for space.with_lock that opens the file if necessary
//! fn with_file<R,F>(space:LockSpace<OsString,File>,filename:Arc<PathBuf>,f: F) -> R
//! 	where F: FnOnce(&mut File) -> R
//! {
//! 	space.with_lock(filename.as_os_str(),
//! 		||OpenOptions::new().read(true).write(true).open(&*filename).unwrap(),f
//! 	)
//! }
//!
//! // Initialize the file
//! let mut filename=env::temp_dir();
//! filename.push("namedlock-test");
//! let filename=Arc::new(filename);
//! File::create(&*filename).unwrap().write_all(b"0").unwrap();
//!
//! let space=LockSpace::<OsString,File>::new(true);
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
use std::mem::drop;

/// A `LockSpace<K,V>` holds many `Mutex<V>`'s, keyed by `K`.
///
/// All accesses to the internal value must go through the `with_lock()` method.
///
/// See the crate documentation for an example.
pub struct LockSpace<K: Eq + Hash,V> {
	// IMPORTANT: To avoid deadlocks, always acquire the inner lock while
	// holding the outer lock. Once the inner lock is acquired, the outer lock
	// can be released.
	names: Arc<Mutex<HashMap<K,Arc<Mutex<V>>>>>,
	auto_cleanup: bool,
}

pub enum LockSpaceRemoveResult {
	Success,
	NotFound,
	PoisonError,
	WouldBlock,
}

// This needs to be implemented manually, since #[derive(Clone)] doesn't
// understand that the type parameters are only used within the Arc<_>
impl<K: Eq + Hash + Clone,V> Clone for LockSpace<K,V> {
	fn clone(&self) -> LockSpace<K,V> {
		LockSpace{names:self.names.clone(),auto_cleanup:self.auto_cleanup}
	}
}

impl<K: Eq + Hash + Clone,V> LockSpace<K,V> {
	/// Create a new LockSpace.
	///
	/// If `auto_cleanup` is true, values will be deleted automatically when
	/// the last lock is released. Otherwise, values will remain in the space
	/// until `try_remove()` returns `Success`.
	pub fn new(auto_cleanup: bool) -> LockSpace<K,V> {
		LockSpace{names:Arc::new(Mutex::new(HashMap::new())),auto_cleanup:auto_cleanup}
	}

	/// Find the object by `key`, or create it by calling `initial` if it does
	/// not exist. Then, call `f` on that object.
	///
	/// ```
	/// let space=namedlock::LockSpace::<String,i32>::new(false);
	///
	/// space.with_lock("test",||0,|i|*i+=1);
	/// assert_eq!(space.with_lock("test",||0,|i|*i),1);
	pub fn with_lock<F,C,R,Q: ?Sized + Hash + Eq>(&self, key: &Q, initial: C, f: F) -> R
		where K: Borrow<Q>, for<'a> &'a Q: Into<K>, /* Take e.g. both &str and &String */
		C: FnOnce() -> V, F: FnOnce(&mut V) -> R
	{
		let target;
		let result;
		{
			let mut guard;
			let mut map=self.names.lock().unwrap(); // Acquire outer lock

			if !map.contains_key(key.borrow()) {
				// Initialize entry if it does not exist
				let mutex=Arc::new(Mutex::new(initial()));
				map.insert(key.into(),mutex);
			}

			target=map.get(key.borrow()).unwrap().clone();
			guard=target.lock().unwrap(); // Acquire inner lock
			drop::<MutexGuard<_>>(map); // Explicitly release outer lock

			result=f(&mut guard);
			drop::<MutexGuard<_>>(guard); // Explicitly release inner lock
		}

		if self.auto_cleanup
		{
			let mut map=self.names.lock().unwrap(); // Acquire outer lock
			Self::try_remove_internal(&mut*map,key,target);
			// Release outer lock
		}
		result
	}

	// IMPORTANT: The caller must hold the outer lock
	// to guard target--and therefore map--against data races
	fn try_remove_internal<Q: ?Sized + Hash + Eq>(map: &mut HashMap<K,Arc<Mutex<V>>>, key: &Q, target: Arc<Mutex<V>>) -> LockSpaceRemoveResult
		where K: Borrow<Q>, /* Take e.g. both &str and &String */
	{
		// This is the "last" reference if strong_count is 2:
		// - map holds 1 reference
		// - target holds 1 reference
		if strong_count(&target)>2 {
			// This means "a remove() function would block", not "calling lock
			// would block".
			return LockSpaceRemoveResult::WouldBlock
		}

		// If we hold the last reference, delete this entry
		match target.try_lock() { // Acquire inner lock
			Ok(_) => match map.remove(key.borrow()) {
				Some(_) => LockSpaceRemoveResult::Success,
				None => LockSpaceRemoveResult::NotFound,
			},
			Err(TryLockError::WouldBlock) => LockSpaceRemoveResult::WouldBlock,
			Err(TryLockError::Poisoned(_)) => LockSpaceRemoveResult::PoisonError,
		} // Release inner lock
	}

	/// Find the object by `key`, then delete it if it is not actively being
	/// used. If it is actually being used, `WouldBlock` will be returned.
	pub fn try_remove<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> LockSpaceRemoveResult
		where K: Borrow<Q>, /* Take e.g. both &str and &String */
	{
		let mut map=self.names.lock().unwrap(); // Acquire outer lock
		let target;
		if let Some(mutex)=map.get(key.borrow()) {
			target=mutex.clone();
		} else {
			return LockSpaceRemoveResult::NotFound
		}
		Self::try_remove_internal(&mut*map,key,target)
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
		let space=Arc::new(LockSpace::<String,bool>::new(true));
		let mut threads=vec![];

		for _ in 0..1000 {
			let space_clone=space.clone();
			threads.push(thread::spawn(move||space_clone.with_lock("test",||false,|b|*b=true)));
		}

		for t in threads.into_iter() {
			t.join().unwrap();
		}

		// This should assert since all threads have exited and the automatic
		// cleanup should have run, which means a fresh value should be
		// generated by the initializer
		space.with_lock("test",||panic!("Intializer must run"),|_|{});
	}
}
