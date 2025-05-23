#![allow(clippy::should_implement_trait)]

use crate::{asm, primitives::Bytes32, storage::Value};
use core::ops::Sub;

/// Account address
///
/// TODO: impl Numeric trait for U256
#[repr(C)]
#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
pub struct U256(Bytes32);

impl U256 {
    /// Returns empty value  
    pub const fn empty() -> Self {
        U256(Bytes32::empty())
    }

    /// u256 add
    #[inline(always)]
    pub fn add(self, other: Self) -> Self {
        unsafe { asm::ext::u256_add(self, other) }
    }

    /// u256 less than
    #[inline(always)]
    pub fn lt(self, other: Self) -> bool {
        unsafe { asm::ext::u256_lt(other, self) }
    }

    /// u256 eq
    #[inline(always)]
    pub fn eq(self, other: Self) -> bool {
        self.0.eq(other.0)
    }

    /// u256 sub
    #[inline(always)]
    pub fn sub(self, other: Self) -> Self {
        unsafe { asm::ext::u256_sub(other, self) }
    }

    /// u256 div
    #[inline(always)]
    pub fn div(self, other: Self) -> Self {
        unsafe { asm::ext::u256_div(self, other) }
    }

    /// max of u256
    #[inline(always)]
    pub fn max() -> Self {
        unsafe { asm::ext::u256_max() }
    }

    pub fn to_bytes32(&self) -> Bytes32 {
        self.0
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn bytes32(&self) -> [u8; 32] {
        self.0 .0 // [u8; 32] in non-WASM
    }

    #[inline(always)]
    pub fn addmod(self, other: Self, modulus: Self) -> Self {
        unsafe { asm::ext::u256_addmod(modulus, other, self) }
    }

    /// Mulmod for U256
    #[inline(always)]
    pub fn mulmod(self, other: Self, modulus: Self) -> Self {
        unsafe { asm::ext::u256_mulmod(modulus, other, self) }
    }
}

impl Sub for U256 {
    type Output = Self;

    #[inline(always)]
    fn sub(self, other: Self) -> Self::Output {
        unsafe { asm::ext::u256_sub(self, other) }
    }
}

impl Value for U256 {
    #[inline(always)]
    fn tload() -> Self {
        Self(unsafe { asm::bytes::tload_bytes32() })
    }

    #[inline(always)]
    fn sload() -> Self {
        Self(unsafe { asm::bytes::sload_bytes32() })
    }

    #[inline(always)]
    fn push(self) {
        unsafe { asm::bytes::push_bytes32(self.0) }
    }

    #[cfg(not(target_family = "wasm"))]
    fn bytes32(&self) -> [u8; 32] {
        self.bytes32() // Delegate to the instance method
    }
}

impl From<u64> for U256 {
    fn from(value: u64) -> Self {
        #[cfg(target_family = "wasm")]
        {
            U256(Bytes32(value as i32))
        }
        #[cfg(not(target_family = "wasm"))]
        {
            // On non-WASM, Bytes32 is [u8; 32]
            let mut bytes = [0u8; 32];
            bytes[24..32].copy_from_slice(&value.to_be_bytes());
            U256(Bytes32(bytes))
        }
    }
}
