// OwnedMutexGuard - Mutex guards that own the Mutex
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

//! Mutex guards that own the Mutex.
//!
//! A standard `MutexGuard` requires the Mutex to live at least as long as the
//! guard. This module contains a new guard type `OwnedMutexGuard`, which
//! guarantees that an `OwnedMutex` stays alive until the guard is released,
//! without any restrictions on the lifetime of the mutex.
//!
//! `Arc<Mutex<_>>`, `Rc<Mutex<_>>` and `Box<Mutex<_>>` implement `OwnedMutex`.
//!
//! The `OwnedMutex.owned_lock` function is used to create a new OwnedMutexGuard.
//!
//! ```
//! use std::sync::{Mutex,Arc};
//! use namedlock::lockresult::LockResult;
//! use namedlock::ownedmutexguard::{OwnedMutex,OwnedMutexGuard};
//!
//! // Note the return value has a lifetime distinct from the input
//! fn get_locked<'a,T: Clone>(input: &T) -> LockResult<OwnedMutexGuard<'a,T,Arc<Mutex<T>>>> {
//! 	Arc::new(Mutex::new(input.clone())).owned_lock()
//! }
//!
//! assert_eq!([0,1,2,3,4,5,6,7,8,9],*get_locked(&[0,1,2,3,4,5,6,7,8,9]).unwrap());
//! ```
//!
//! ## License
//! OwnedMutexGuard - Copyright (C) 2015  Jethro G. Beekman
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

#[cfg(all(feature="std",not(feature="spin")))] use std::sync::{Mutex,MutexGuard};
#[cfg(feature="spin")] use spin::{Mutex,MutexGuard};
use core::ops::{Deref,DerefMut};

#[cfg(feature="std")] use std::rc::Rc;
#[cfg(feature="std")] use std::sync::Arc;
#[cfg(not(feature="std"))] use alloc::boxed::Box;
#[cfg(not(feature="std"))] use alloc::rc::Rc;
#[cfg(not(feature="std"))] use alloc::arc::Arc;

use lockresult::LockResult as Result;
use private::IntoResult;

/// An RAII implementation of a "scoped lock" of a mutex. When this structure
/// is dropped (falls out of scope), the lock will be unlocked, and the
/// owner of the Mutex will be dropped.
///
/// Alternatively, call `into_inner` to drop the guard and reclaim the owner.
///
/// The data protected by the mutex can be accessed through this guard via its
/// Deref and DerefMut implementations.
pub struct OwnedMutexGuard<'a, T: 'a, M: OwnedMutex<T>> {
	owned_mutex: Option<M>,
	guard: Option<MutexGuard<'a,T>>,
}

impl<'a, T: 'a, M: OwnedMutex<T>> Deref for OwnedMutexGuard<'a,T,M> {
	type Target = T;
	fn deref<'b>(&'b self) -> &'b T {
		// This is always Some, because it's initialized as Some, and only drop() and into_inner() turn it into None
		match self.guard {
			Some(ref value) => &value,
			None => unreachable!(), // to be replace with std::intrinsics::unreachable once stable
		}
	}
}

impl<'a, T:'a, M: OwnedMutex<T>> DerefMut for OwnedMutexGuard<'a,T,M> {
	fn deref_mut<'b>(&'b mut self) -> &'b mut T {
		// This is always Some, because it's initialized as Some, and only drop() and into_inner() turn it into None
		match self.guard {
			Some(ref mut value) => unsafe{&mut*(value.deref_mut() as *mut _) as &'b mut T},
			None => unreachable!(), // to be replace with std::intrinsics::unreachable once stable
		}
	}
}

impl<'a, T:'a, M: OwnedMutex<T>> Drop for OwnedMutexGuard<'a,T,M> {
	fn drop(&mut self) {
		self.guard=None;
	}
}

impl<'a, T: 'a, M: OwnedMutex<T>> OwnedMutexGuard<'a,T,M> {
	/// Drops the guard and returns the associated `OwnedMutex`
	pub fn into_inner(mut self) -> M {
		self.guard=None;
		// This is always Some, because it's initialized as Some, and only drop() or this turns it into None
		self.owned_mutex.take().unwrap()
	}
}

/// Implements the functions to obtain `OwnedMutexGuard`s.
///
/// This trait must only be implemented for types for which the memory address
/// of the value reachable via Deref remains identical even if self gets moved.
pub unsafe trait OwnedMutex<T>: Sized + Deref<Target=Mutex<T>> {
	/// Acquires an `OwnedMutex`, blocking the current thread until it is able to do so.
	///
	/// This function will block the local thread until it is available to acquire the mutex.
	/// Upon returning, the thread is the only thread with the mutex held. An RAII guard is
	/// returned to allow scoped unlock of the lock. When the guard goes out of scope, the
	/// mutex will be unlocked, and the OwnedMutex will be dropped.
	// Unsafety explanation:
	// The MutexGuard holds a reference to it's Mutex. As such, the mutex must stay alive
	// at that address until the guard drops. We guarantee this by storing the mutex
	// alongside the guard.
	//
	// In particular, we know that our reference to the mutex can be safely converted to
	// lifetime 'a since we will be storing the OwnedMutex in a structure with the same
	// lifetime 'a.
	fn owned_lock<'a>(self) -> Result<OwnedMutexGuard<'a,T,Self>> where Self: 'a {
		let guard=try!(unsafe{&*(&self as *const _) as &'a Mutex<T>}.lock().into_result());
		return Ok(OwnedMutexGuard{owned_mutex:Some(self),guard:Some(guard)});
	}
}

unsafe impl<T> OwnedMutex<T> for Box<Mutex<T>> {}
unsafe impl<T> OwnedMutex<T> for Rc<Mutex<T>> {}
unsafe impl<T> OwnedMutex<T> for Arc<Mutex<T>> {}
