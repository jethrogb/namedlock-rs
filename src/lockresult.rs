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

//! A `Result` type very similar to `std::sync::LockResult`.
use std::fmt;
use std::marker::PhantomData;
// Use PhantomData to mimic type signature
pub struct PoisonError<T>(PhantomData<T>);
impl<T> PoisonError<T> {
	pub fn new() -> PoisonError<T> {
		PoisonError(PhantomData)
	}
}
impl<T> fmt::Debug for PoisonError<T> {
	fn fmt(&self, fmt: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		fmt.write_str("PoisonError")
	}
}
/// A `Result` type very similar to `std::sync::LockResult`.
///
/// We can't use sync's LockResult because we can't map it's PoisonError inner
/// guard
pub type LockResult<T> = Result<T,PoisonError<T>>;
