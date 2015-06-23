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

use std::sync::Arc;
use std::borrow::Borrow;
use std::hash::{Hash,Hasher};

#[derive(Clone)]
pub struct ArcHashKey<T>(pub Arc<T>);

impl<T: PartialEq> PartialEq for ArcHashKey<T> {
	fn eq(&self, other: &ArcHashKey<T>) -> bool {
		self.0.eq(&other.0)
	}
}

impl<T: Eq> Eq for ArcHashKey<T> { }

impl<T: Hash> Hash for ArcHashKey<T> {
	fn hash<H: Hasher>(&self, state: &mut H)  {
		self.0.hash(state)
	}
}

/// See the `LockSpace` documentation on "Key parameters" for more information
/// on this macro.
#[macro_export]
macro_rules! ahk_chain_borrow {
    ( $( $t:ty : ( $( $param:ident , )* ) , )* ) => {
		$(
			impl<$($param,)* BorrowTarget: ?Sized> Borrow<BorrowTarget> for $crate::archashkey::ArcHashKey<$t>
				where $t: Borrow<BorrowTarget>
			{
				fn borrow(&self) -> &BorrowTarget {
					let a: &$t=self.0.borrow();
					a.borrow()
				}
			}
		)*
    };
}

use std::path::PathBuf;
use std::ffi::OsString;
use std::borrow::Cow;
use std::borrow::ToOwned;

ahk_chain_borrow! {
	String: (),
	OsString: (),
	PathBuf: (),
	Vec<T>: ( T, ),
}

impl<'a, B: ?Sized + ToOwned, T: ?Sized> Borrow<T> for ArcHashKey<Cow<'a, B>>
	where Cow<'a, B>: Borrow<T>, B::Owned: 'a,
{
	fn borrow(&self) -> &T {
		let a: &Cow<'a, B> = self.0.borrow();
		a.borrow()
	}
}
