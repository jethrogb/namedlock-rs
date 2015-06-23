// arcmutexguard - Mutex guards that can outlive the Mutex
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

//! Mutex guards that can outlive the Mutex.
//!
//! A standard `MutexGuard` requires the Mutex to live at least as long as the
//! guard. This module contains a new guard type `ArcMutexGuard`, which
//! guarantees that an `Arc<Mutex<_>>` stays alive until the guard is released,
//! without any restrictions on the lifetime of the mutex.
//!
//! The `arc_mutex_lock` function is used to create a new ArcMutexGuard.
//!
//! ```
//! use std::sync::{Mutex,Arc};
//! use namedlock::lockresult::LockResult;
//! use namedlock::arcmutexguard::{ArcMutexGuard,arc_mutex_lock};
//!
//! // Note the return value has a lifetime distinct from the input
//! fn get_locked<'a,T: Clone>(input: &T) -> LockResult<ArcMutexGuard<'a,T>> {
//! 	let a=Arc::new(Mutex::new(input.clone()));
//! 	arc_mutex_lock(a)
//! }
//!
//! assert_eq!([0,1,2,3,4,5,6,7,8,9],*get_locked(&[0,1,2,3,4,5,6,7,8,9]).unwrap());
//! ```
//!
//! ## License
//! arcmutexguard - Copyright (C) 2015  Jethro G. Beekman
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

use std::sync::{Arc,Mutex,MutexGuard};
use std::ops::{Deref,DerefMut};

use lockresult::*;

/// An RAII implementation of a "scoped lock" of a mutex. When this structure
/// is dropped (falls out of scope), the lock will be unlocked, and the
/// reference count to the Mutex will be decreased by 1.
///
/// The data protected by the mutex can be accessed through this guard via its
/// Deref and DerefMut implementations.
// The mutex in here is critical to the memory safety of this construct. Don't
// complain about it's unuse.
#[allow(dead_code)]
pub struct ArcMutexGuard<'a, T: 'a> {
	mutex: Arc<Mutex<T>>,
	guard: Option<MutexGuard<'a,T>>,
}

impl<'a, T: 'a> Deref for ArcMutexGuard<'a,T> {
	type Target = T;
	fn deref<'b>(&'b self) -> &'b T {
		// This is always Some, because it's initialized as Some, and only drop() turns it into None
		match self.guard {
			Some(ref value) => &value,
			None => unreachable!(), // to be replace with std::intrinsics::unreachable once stable
		}
	}
}

impl<'a, T:'a> DerefMut for ArcMutexGuard<'a,T> {
	fn deref_mut<'b>(&'b mut self) -> &'b mut T {
		// This is always Some, because it's initialized as Some, and only drop() turns it into None
		match self.guard {
			Some(ref mut value) => unsafe{&mut*(value.deref_mut() as *mut _) as &'b mut T},
			None => unreachable!(), // to be replace with std::intrinsics::unreachable once stable
		}
	}
}

impl<'a, T:'a> Drop for ArcMutexGuard<'a,T> {
	fn drop(&mut self) {
		self.guard=None;
	}
}

/// Acquires an `Arc<Mutex<_>>`, blocking the current thread until it is able to do so.
///
/// This function will block the local thread until it is available to acquire the mutex.
/// Upon returning, the thread is the only thread with the mutex held. An RAII guard is
/// returned to allow scoped unlock of the lock. When the guard goes out of scope, the
/// mutex will be unlocked, and the Arc will be unreferenced.
// Unsafety explanation:
// The MutexGuard holds a reference to it's Mutex. As such, the mutex must stay alive
// at that address until the guard drops. We guarantee this by storing the mutex
// alongside the guard.
//
// In particular, we know that our reference to the mutex can be safely converted to
// lifetime 'a since we will be storing the Arc<Mutex<_>> in a structure with the same
// lifetime 'a.
pub fn arc_mutex_lock<'a,T>(mutex: Arc<Mutex<T>>) -> LockResult<ArcMutexGuard<'a,T>> {
	let lock_result=unsafe{&*(&mutex as *const _) as &'a Mutex<T>}.lock();
	match lock_result {
		Ok(guard) => Ok(ArcMutexGuard{mutex:mutex,guard:Some(guard)}),
		Err(_) => Err(PoisonError::new())
	}
}
