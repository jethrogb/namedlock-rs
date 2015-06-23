// arcdebug - Print backtraces when an Arc is cloned or dropped.
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

// in your crate root, put : #![feature(std_misc,unmarked_api)]
use std::rt::backtrace;

use std::thread;
use std::sync::Arc;
use std::ops::Deref;
use std::io::{Write,stdout};

pub struct ArcDebug<T>(pub Arc<T>);

impl<T> ArcDebug<T> {
	pub fn new(data: T) -> ArcDebug<T> {
		ArcDebug(Arc::new(data))
	}
}

impl<T> Deref for ArcDebug<T> {
	type Target = T;
	fn deref(&self) -> &T {
		self.0.deref()
	}
}

impl<T> Drop for ArcDebug<T> {
	fn drop(&mut self) {
		let stdout=stdout();
		let mut lock=stdout.lock();
		writeln!(lock,"Thread {}: Dropping Arc with inner {:?}",thread::current().name().unwrap_or("anon"),(*self).deref() as *const T).unwrap();
		backtrace::write(&mut lock).unwrap();
	}
}

impl<T> Clone for ArcDebug<T> {
	fn clone(&self) -> ArcDebug<T> {
		let stdout=stdout();
		let mut lock=stdout.lock();
		writeln!(lock,"Thread {}: Cloning Arc with inner {:?}",thread::current().name().unwrap_or("anon"),(*self).deref() as *const T).unwrap();
		backtrace::write(&mut lock).unwrap();
		ArcDebug(self.0.clone())
	}
}
