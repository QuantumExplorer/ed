#![feature(fundamental)]

//! *`ed` is a minimalist crate for deterministic binary encodings.*
//!
//! ## Overview
//!
//! This crate provides `Encode` and `Decode` traits which can be implemented for any
//! type that can be converted to or from bytes, and implements these traits for
//! many built-in Rust types. It also provides derive macros so that `Encode`
//! and `Decode` can be easily derived for structs.
//!
//! `ed` is far simpler than `serde` because it does not attempt to create an
//! abstraction which allows arbitrary kinds of encoding (JSON, MessagePack,
//! etc.), and instead forces focuses on binary encodings. It is also
//! significantly faster than [`bincode`](https://docs.rs/bincode), the leading
//! binary `serde` serializer.
//!
//! One aim of `ed` is to force top-level type authors to design their own
//! encoding, rather than attempting to provide a one-size-fits-all encoding
//! scheme. This lets users of `ed` be sure their encodings are as effiient as
//! possible, and makes it easier to understand the encoding for compatability
//! in other languages or libraries (contrasted with something like `bincode`,
//! where it is not obvious how a type is being encoded without understanding
//! the internals of `bincode`).
//!
//! Another property of this crate is a focus on determinism (important for
//! cryptographically hashed types) - built-in encodings are always big-endian
//! and there are no provided encodings for floating point numbers or `usize`.
//!
//! ## Usage
//!
//! ```rust
//! #![feature(trivial_bounds)]
//! use ed::{Encode, Decode};
//!
//! # fn main() -> ed::Result<()> {
//! // traits are implemented for built-in types
//! let bytes = 123u32.encode()?; // `bytes` is a Vec<u8>
//! let n = u32::decode(bytes.as_slice())?; // `n` is a u32
//!
//! // derive macros are available
//! #[derive(Encode, Decode)]
//! # #[derive(PartialEq, Eq, Debug)]
//! struct Foo {
//!   bar: (u32, u32),
//!   baz: Vec<u8>
//! }
//!
//! // encoding and decoding can be done in-place to reduce allocations
//!
//! let mut bytes = vec![0xba; 40];
//! let mut foo = Foo {
//!   bar: (0, 0),
//!   baz: Vec::with_capacity(32)
//! };
//!
//! // in-place decode, re-using pre-allocated `foo.baz` vec
//! foo.decode_into(bytes.as_slice())?;
//! assert_eq!(foo, Foo {
//!   bar: (0xbabababa, 0xbabababa),
//!   baz: vec![0xba; 32]
//! });
//!
//! // in-place encode, into pre-allocated `bytes` vec
//! bytes.clear();
//! foo.encode_into(&mut bytes)?;
//!
//! # Ok(())
//! # }
//! ```

use std::convert::TryInto;
use std::io::{Read, Write};

pub use ed_derive::*;

/// An enum that defines the `ed` error types.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Unexpected byte: {0}")]
    UnexpectedByte(u8),
    #[error("Unencodable variant")]
    UnencodableVariant,
    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

/// A Result bound to the standard `ed` error type.
pub type Result<T> = std::result::Result<T, Error>;

/// A trait for values that can be encoded into bytes deterministically.
#[fundamental]
pub trait Encode {
    /// Writes the encoded representation of the value to the destination
    /// writer. Can error due to either a write error from `dest`, or an
    /// encoding error for types where invalid values are possible.
    ///
    /// It may be more convenient to call [`encode`](#method.encode) which
    /// returns bytes, however `encode_into` will often be more efficient since
    /// it can write the encoding without necessarily allocating a new
    /// `Vec<u8>`.
    fn encode_into<W: Write>(&self, dest: &mut W) -> Result<()>;

    /// Calculates the length of the encoding for this value. Can error for
    /// types where invalid values are possible.
    fn encoding_length(&self) -> Result<usize>;

    /// Returns the encoded representation of the value as a `Vec<u8>`.
    ///
    /// While this method is convenient, it will often be more efficient to call
    /// [`encode_into`](#method.encode_into) since `encode` usually involves
    /// allocating a new `Vec<u8>`.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn encode(&self) -> Result<Vec<u8>> {
        let length = self.encoding_length()?;
        let mut bytes = Vec::with_capacity(length);
        self.encode_into(&mut bytes)?;
        Ok(bytes)
    }
}

/// A trait for values that can be decoded from bytes deterministically.
#[fundamental]
pub trait Decode: Sized {
    /// Reads bytes from the reader and returns the decoded value.
    ///
    /// When possible, calling [`decode_into`](#method.decode_into) will often
    /// be more efficient since it lets the caller reuse memory to avoid
    /// allocating for fields with types such as `Vec<T>`.
    fn decode<R: Read>(input: R) -> Result<Self>;

    /// Reads bytes from the reader and mutates self to the decoded value.
    ///
    /// This is often more efficient than calling [`decode`](#method.decode)
    /// when reading fields with heap-allocated types such as `Vec<T>` since it
    /// can reuse the memory already allocated in self.
    ///
    /// When possible, implementations should recursively call `decode_into` on
    /// any child fields.
    ///
    /// The default implementation of `decode_into` simply calls
    /// [`decode`](#method.decode) for ease of implementation, but should be
    /// overridden when in-place decoding is possible.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn decode_into<R: Read>(&mut self, input: R) -> Result<()> {
        let value = Self::decode(input)?;
        *self = value;
        Ok(())
    }
}

/// A type is `Terminated` the length of the value being read can be determined
/// when decoding.
///
/// Since `Terminated` is an auto trait, it is automatically present for any
/// type made of fields which are all `Terminated`.
///
/// Consider a type like `u32` - it is always 4 bytes long. If a slice of length
/// 5 was passed to its `decode` method, it would know to stop reading after the
/// 4th byte, which means it is `Terminated`.
///
/// For an example of something which is NOT terminated, consider `Vec<u8>`. Its
/// encoding and decoding do not use a length prefix or end with a null byte, so
/// `decode` would have no way to know where to stop reading.
pub trait Terminated {}

macro_rules! int_impl {
    ($type:ty, $length:expr) => {
        impl Encode for $type {
            #[doc = "Encodes the integer as fixed-size big-endian bytes."]
            #[inline]
            fn encode_into<W: Write>(&self, dest: &mut W) -> Result<()> {
                let bytes = self.to_be_bytes();
                dest.write_all(&bytes[..])?;
                Ok(())
            }

            #[doc = "Returns the size of the integer in bytes. Will always"]
            #[doc = " return an `Ok` result."]
            #[inline]
            fn encoding_length(&self) -> Result<usize> {
                Ok($length)
            }
        }

        impl Decode for $type {
            #[doc = "Decodes the integer from fixed-size big-endian bytes."]
            #[inline]
            fn decode<R: Read>(mut input: R) -> Result<Self> {
                let mut bytes = [0; $length];
                input.read_exact(&mut bytes[..])?;
                Ok(Self::from_be_bytes(bytes))
            }
        }

        impl Terminated for $type {}
    };
}

int_impl!(u8, 1);
int_impl!(u16, 2);
int_impl!(u32, 4);
int_impl!(u64, 8);
int_impl!(u128, 16);
int_impl!(i8, 1);
int_impl!(i16, 2);
int_impl!(i32, 4);
int_impl!(i64, 8);
int_impl!(i128, 16);

impl Encode for bool {
    /// Encodes the boolean as a single byte: 0 for false or 1 for true.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn encode_into<W: Write>(&self, dest: &mut W) -> Result<()> {
        let bytes = [*self as u8];
        dest.write_all(&bytes[..])?;
        Ok(())
    }

    /// Always returns Ok(1).
    #[inline]
    #[cfg_attr(test, mutate)]
    fn encoding_length(&self) -> Result<usize> {
        Ok(1)
    }
}

impl Decode for bool {
    /// Decodes the boolean from a single byte: 0 for false or 1 for true.
    /// Errors for any other value.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn decode<R: Read>(mut input: R) -> Result<Self> {
        let mut buf = [0; 1];
        input.read_exact(&mut buf[..])?;
        match buf[0] {
            0 => Ok(false),
            1 => Ok(true),
            byte => Err(Error::UnexpectedByte(byte)),
        }
    }
}

impl Terminated for bool {}

impl<T: Encode> Encode for Option<T> {
    /// Encodes as a 0 byte for `None`, or as a 1 byte followed by the encoding of
    /// the inner value for `Some`.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn encode_into<W: Write>(&self, dest: &mut W) -> Result<()> {
        match self {
            None => dest.write_all(&[0]).map_err(Error::IOError),
            Some(value) => {
                dest.write_all(&[1]).map_err(Error::IOError)?;
                value.encode_into(dest)
            }
        }
    }

    /// Length will be 1 for `None`, or 1 plus the encoding length of the inner
    /// value for `Some`.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn encoding_length(&self) -> Result<usize> {
        match self {
            None => Ok(1),
            Some(value) => Ok(1 + value.encoding_length()?),
        }
    }
}

impl<T: Decode> Decode for Option<T> {
    /// Decodes a 0 byte as `None`, or a 1 byte followed by the encoding of the
    /// inner value as `Some`. Errors for all other values.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn decode<R: Read>(input: R) -> Result<Self> {
        let mut option: Option<T> = None;
        option.decode_into(input)?;
        Ok(option)
    }

    /// Decodes a 0 byte as `None`, or a 1 byte followed by the encoding of the
    /// inner value as `Some`. Errors for all other values.
    //
    // When the first byte is 1 and self is `Some`, `decode_into` will be called
    // on the inner type. When the first byte is 1 and self is `None`, `decode`
    // will be called for the inner type.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn decode_into<R: Read>(&mut self, mut input: R) -> Result<()> {
        let mut byte = [0; 1];
        input.read_exact(&mut byte[..])?;

        match byte[0] {
            0 => *self = None,
            1 => match self {
                None => *self = Some(T::decode(input)?),
                Some(value) => value.decode_into(input)?,
            },
            byte => {
                return Err(Error::UnexpectedByte(byte));
            }
        };

        Ok(())
    }
}

impl<T: Terminated> Terminated for Option<T> {}

impl Encode for () {
    /// Encoding a unit tuple is a no-op.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn encode_into<W: Write>(&self, _: &mut W) -> Result<()> {
        Ok(())
    }

    /// Always returns Ok(0).
    #[inline]
    #[cfg_attr(test, mutate)]
    fn encoding_length(&self) -> Result<usize> {
        Ok(0)
    }
}

impl Decode for () {
    /// Returns a unit tuple without reading any bytes.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn decode<R: Read>(_: R) -> Result<Self> {
        Ok(())
    }
}

impl Terminated for () {}

macro_rules! tuple_impl {
    ($( $type:ident ),*; $last_type:ident) => {
        impl<$($type: Encode + Terminated,)* $last_type: Encode> Encode for ($($type,)* $last_type,) {
            #[doc = "Encodes the fields of the tuple one after another, in"]
            #[doc = " order."]
            #[allow(non_snake_case, unused_mut)]
            #[inline]
            fn encode_into<W: Write>(&self, mut dest: &mut W) -> Result<()> {
                let ($($type,)* $last_type,) = self;
                $($type.encode_into(&mut dest)?;)*
                $last_type.encode_into(dest)
            }

            #[doc = "Returns the sum of the encoding lengths of the fields of"]
            #[doc = " the tuple."]
            #[allow(non_snake_case)]
            #[allow(clippy::needless_question_mark)]
            #[inline]
            fn encoding_length(&self) -> Result<usize> {
                let ($($type,)* $last_type,) = self;
                Ok(
                    $($type.encoding_length()? +)*
                    $last_type.encoding_length()?
                )
            }
        }

        impl<$($type: Decode + Terminated,)* $last_type: Decode> Decode for ($($type,)* $last_type,) {
            #[doc = "Decodes the fields of the tuple one after another, in"]
            #[doc = " order."]
            #[allow(unused_mut)]
            #[inline]
            fn decode<R: Read>(mut input: R) -> Result<Self> {
                Ok((
                    $($type::decode(&mut input)?,)*
                    $last_type::decode(input)?,
                ))
            }

            #[doc = "Decodes the fields of the tuple one after another, in"]
            #[doc = " order."]
            #[doc = ""]
            #[doc = "Recursively calls `decode_into` for each field."]
            #[allow(non_snake_case, unused_mut)]
            #[inline]
            fn decode_into<R: Read>(&mut self, mut input: R) -> Result<()> {
                let ($($type,)* $last_type,) = self;
                $($type.decode_into(&mut input)?;)*
                $last_type.decode_into(input)?;
                Ok(())
            }
        }

        impl<$($type: Terminated,)* $last_type: Terminated> Terminated for ($($type,)* $last_type,) {}
    }
}

tuple_impl!(; A);
tuple_impl!(A; B);
tuple_impl!(A, B; C);
tuple_impl!(A, B, C; D);
tuple_impl!(A, B, C, D; E);
tuple_impl!(A, B, C, D, E; F);
tuple_impl!(A, B, C, D, E, F; G);
tuple_impl!(A, B, C, D, E, F, G; H);
tuple_impl!(A, B, C, D, E, F, G, H; I);
tuple_impl!(A, B, C, D, E, F, G, H, I; J);
tuple_impl!(A, B, C, D, E, F, G, H, I, J; K);
tuple_impl!(A, B, C, D, E, F, G, H, I, J, K; L);

impl<T: Encode + Terminated, const N: usize> Encode for [T; N] {
    #[inline]
    fn encode_into<W: Write>(&self, mut dest: &mut W) -> Result<()> {
        for element in self[..].iter() {
            element.encode_into(&mut dest)?;
        }
        Ok(())
    }

    #[inline]
    fn encoding_length(&self) -> Result<usize> {
        let mut sum = 0;
        for element in self[..].iter() {
            sum += element.encoding_length()?;
        }
        Ok(sum)
    }
}

impl<T: Decode + Terminated, const N: usize> Decode for [T; N] {
    #[allow(unused_variables, unused_mut)]
    #[inline]
    fn decode<R: Read>(mut input: R) -> Result<Self> {
        let mut v: Vec<T> = Vec::with_capacity(N);
        for i in 0..N {
            v.push(T::decode(&mut input)?);
        }
        Ok(v.try_into()
            .unwrap_or_else(|v: Vec<T>| panic!("Input Vec not of length {}", N)))
    }

    #[inline]
    fn decode_into<R: Read>(&mut self, mut input: R) -> Result<()> {
        for item in self.iter_mut().take(N) {
            T::decode_into(item, &mut input)?;
        }
        Ok(())
    }
}

impl<T: Terminated, const N: usize> Terminated for [T; N] {}

impl<T: Encode + Terminated> Encode for Vec<T> {
    #[doc = "Encodes the elements of the vector one after another, in order."]
    #[inline]
    fn encode_into<W: Write>(&self, dest: &mut W) -> Result<()> {
        for element in self.iter() {
            element.encode_into(dest)?;
        }
        Ok(())
    }

    #[doc = "Returns the sum of the encoding lengths of all elements."]
    #[cfg_attr(test, mutate)]
    #[inline]
    fn encoding_length(&self) -> Result<usize> {
        let mut sum = 0;
        for element in self.iter() {
            sum += element.encoding_length()?;
        }
        Ok(sum)
    }
}

impl<T: Decode + Terminated> Decode for Vec<T> {
    #[doc = "Decodes the elements of the vector one after another, in order."]
    #[cfg_attr(test, mutate)]
    #[inline]
    fn decode<R: Read>(input: R) -> Result<Self> {
        let mut vec = Vec::with_capacity(128);
        vec.decode_into(input)?;
        Ok(vec)
    }

    #[doc = "Encodes the elements of the vector one after another, in order."]
    #[doc = ""]
    #[doc = "Recursively calls `decode_into` for each element."]
    #[cfg_attr(test, mutate)]
    #[inline]
    fn decode_into<R: Read>(&mut self, mut input: R) -> Result<()> {
        let old_len = self.len();

        let mut bytes = Vec::with_capacity(256);
        input.read_to_end(&mut bytes)?;

        let mut slice = bytes.as_slice();
        let mut i = 0;
        while !slice.is_empty() {
            if i < old_len {
                self[i].decode_into(&mut slice)?;
            } else {
                let el = T::decode(&mut slice)?;
                self.push(el);
            }

            i += 1;
        }

        if i < old_len {
            self.truncate(i);
        }

        Ok(())
    }
}

impl<T: Encode + Terminated> Encode for [T] {
    #[doc = "Encodes the elements of the slice one after another, in order."]
    #[cfg_attr(test, mutate)]
    #[inline]
    fn encode_into<W: Write>(&self, mut dest: &mut W) -> Result<()> {
        for element in self[..].iter() {
            element.encode_into(&mut dest)?;
        }
        Ok(())
    }

    #[doc = "Returns the sum of the encoding lengths of all elements."]
    #[cfg_attr(test, mutate)]
    #[inline]
    fn encoding_length(&self) -> Result<usize> {
        let mut sum = 0;
        for element in self[..].iter() {
            sum += element.encoding_length()?;
        }
        Ok(sum)
    }
}

impl<T: Encode> Encode for Box<T> {
    #[doc = "Encodes the inner value."]
    #[cfg_attr(test, mutate)]
    #[inline]
    fn encode_into<W: Write>(&self, dest: &mut W) -> Result<()> {
        (**self).encode_into(dest)
    }

    #[doc = "Returns the encoding length of the inner value."]
    #[cfg_attr(test, mutate)]
    #[inline]
    fn encoding_length(&self) -> Result<usize> {
        (**self).encoding_length()
    }
}

impl<T: Decode> Decode for Box<T> {
    #[doc = "Decodes the inner value into a new Box."]
    #[cfg_attr(test, mutate)]
    #[inline]
    fn decode<R: Read>(input: R) -> Result<Self> {
        T::decode(input).map(|v| v.into())
    }

    #[doc = "Decodes the inner value into the existing Box."]
    #[doc = ""]
    #[doc = "Recursively calls `decode_into` on the inner value."]
    #[cfg_attr(test, mutate)]
    #[inline]
    fn decode_into<R: Read>(&mut self, input: R) -> Result<()> {
        (**self).decode_into(input)
    }
}

impl<T> Encode for std::marker::PhantomData<T> {
    /// Encoding PhantomData is a no-op.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn encode_into<W: Write>(&self, _: &mut W) -> Result<()> {
        Ok(())
    }

    /// Always returns Ok(0).
    #[inline]
    #[cfg_attr(test, mutate)]
    fn encoding_length(&self) -> Result<usize> {
        Ok(0)
    }
}

impl<T> Decode for std::marker::PhantomData<T> {
    /// Returns a PhantomData without reading any bytes.
    #[inline]
    #[cfg_attr(test, mutate)]
    fn decode<R: Read>(_: R) -> Result<Self> {
        Ok(Self {})
    }
}

impl<T> Terminated for std::marker::PhantomData<T> {}

#[cfg(test)]
use mutagen::mutate;
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn encode_decode_u8() {
        let value = 0x12u8;
        let bytes = value.encode().unwrap();
        assert_eq!(bytes.as_slice(), &[0x12]);
        let decoded_value = u8::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, value);
    }

    #[test]
    fn encode_decode_u64() {
        let value = 0x1234567890u64;
        let bytes = value.encode().unwrap();
        assert_eq!(bytes.as_slice(), &[0, 0, 0, 0x12, 0x34, 0x56, 0x78, 0x90]);
        let decoded_value = u64::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, value);
    }

    #[test]
    fn encode_decode_option() {
        let value = Some(0x1234567890u64);
        let bytes = value.encode().unwrap();
        assert_eq!(
            bytes.as_slice(),
            &[1, 0, 0, 0, 0x12, 0x34, 0x56, 0x78, 0x90]
        );
        let decoded_value: Option<u64> = Decode::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, value);

        let value: Option<u64> = None;
        let bytes = value.encode().unwrap();
        assert_eq!(bytes.as_slice(), &[0]);
        let decoded_value: Option<u64> = Decode::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, None);
    }

    #[test]
    fn encode_decode_tuple() {
        let value: (u16, u16) = (1, 2);
        let bytes = value.encode().unwrap();
        assert_eq!(bytes.as_slice(), &[0, 1, 0, 2]);
        let decoded_value: (u16, u16) = Decode::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, value);

        let value = ();
        let bytes = value.encode().unwrap();
        assert_eq!(bytes.as_slice().len(), 0);
        let decoded_value: () = Decode::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, value);
    }

    #[test]
    fn encode_decode_array() {
        let value: [u16; 4] = [1, 2, 3, 4];
        let bytes = value.encode().unwrap();
        assert_eq!(bytes.as_slice(), &[0, 1, 0, 2, 0, 3, 0, 4]);
        let decoded_value: [u16; 4] = Decode::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, value);
    }

    #[test]
    #[should_panic(expected = "failed to fill whole buffer")]
    fn encode_decode_array_eof_length() {
        let bytes = [0, 1, 0, 2, 0, 3];
        let _: [u16; 4] = Decode::decode(&bytes[..]).unwrap();
    }

    #[test]
    #[should_panic(expected = "failed to fill whole buffer")]
    fn encode_decode_array_eof_element() {
        let bytes = [0, 1, 0, 2, 0, 3, 0];
        let _: [u16; 4] = Decode::decode(&bytes[..]).unwrap();
    }

    #[test]
    fn encode_decode_vec() {
        let value: Vec<u16> = vec![1, 2, 3, 4];
        let bytes = value.encode().unwrap();
        assert_eq!(bytes.as_slice(), &[0, 1, 0, 2, 0, 3, 0, 4]);
        let decoded_value: Vec<u16> = Decode::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, value);
    }

    #[test]
    #[should_panic(expected = "failed to fill whole buffer")]
    fn encode_decode_vec_eof_element() {
        let bytes = [0, 1, 0, 2, 0, 3, 0];
        let _: Vec<u16> = Decode::decode(&bytes[..]).unwrap();
    }

    #[test]
    fn test_encode_bool() {
        let value: bool = true;
        let bytes = value.encode().unwrap();
        assert_eq!(bytes.as_slice(), &[1]);
    }

    #[test]
    fn test_encoding_length_bool() {
        let value: bool = true;
        let enc_length = value.encoding_length().unwrap();
        assert!(enc_length == 1);
    }
    #[test]
    fn test_decode_bool_true() {
        let bytes = vec![1];
        let decoded_value: bool = Decode::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, true);
    }

    #[test]
    fn test_decode_bool_false() {
        let bytes = vec![0];
        let decoded_value: bool = Decode::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, false);
    }

    #[test]
    fn test_decode_bool_bail() {
        let bytes = vec![42];
        let result: Result<bool> = Decode::decode(bytes.as_slice());
        assert_eq!(result.unwrap_err().to_string(), "Unexpected byte: 42");
    }

    #[test]
    fn test_encode_decode_phantom_data() {
        use std::marker::PhantomData;
        let pd: PhantomData<u8> = PhantomData;
        let bytes = pd.encode().unwrap();
        assert_eq!(bytes.len(), 0);
        let decoded_value: PhantomData<u8> = Decode::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded_value, PhantomData);
    }

    #[test]
    fn test_default_decode() {
        struct Foo {
            bar: u8,
        }

        impl Decode for Foo {
            fn decode<R: Read>(_input: R) -> Result<Self> {
                Ok(Foo { bar: 42 })
            }
        }

        let bytes = vec![42, 12, 68];
        let mut foo: Foo = Foo { bar: 41 };
        foo.decode_into(bytes.as_slice()).unwrap();
        assert_eq!(foo.bar, 42);
    }

    #[test]
    fn test_option_encode_into() {
        let option = Some(0x12u8);
        let mut vec: Vec<u8> = vec![];
        option.encode_into(&mut vec).unwrap();
        assert_eq!(vec, vec![1, 18]);
    }

    #[test]
    fn test_option_encoding_length() {
        let val = 0x12u8;
        let option = Some(val);
        let option_length = option.encoding_length().unwrap();
        let val_length = val.encoding_length().unwrap();
        assert!(option_length == val_length + 1);
    }
    #[test]
    fn test_option_none_encode_into() {
        let option: Option<u8> = None;
        let mut vec: Vec<u8> = vec![];
        option.encode_into(&mut vec).unwrap();
        assert_eq!(vec, vec![0]);
    }

    #[test]
    fn test_option_none_encoding_length() {
        let option: Option<u8> = None;
        let length = option.encoding_length().unwrap();
        assert!(length == 1);
    }

    #[test]
    fn test_bail_option_decode_into() {
        let mut option: Option<u8> = Some(42);
        let bytes = vec![42];
        let err = option.decode_into(bytes.as_slice()).unwrap_err();
        assert_eq!(err.to_string(), "Unexpected byte: 42");
    }

    #[test]
    fn test_some_option_decode_into() {
        let mut option: Option<u8> = Some(0);
        let bytes = vec![1, 0x12u8];
        option.decode_into(bytes.as_slice()).unwrap();
        assert_eq!(option.unwrap(), 18);
    }

    #[test]
    fn test_vec_decode_into() {
        let mut vec: Vec<u8> = vec![42, 42, 42];
        let bytes = vec![12, 13];
        vec.decode_into(bytes.as_slice()).unwrap();
        assert_eq!(vec, vec![12, 13]);
    }

    #[test]
    fn test_vec_encoding_length() {
        let forty_two: u8 = 42;
        let vec: Vec<u8> = vec![42, 42, 42];
        let vec_length = vec.encoding_length().unwrap();
        let indv_num_length = forty_two.encoding_length().unwrap();
        assert!(vec_length == indv_num_length * 3);
    }

    #[test]
    fn test_box_encoding_length() {
        let forty_two = Box::new(42);
        let length = forty_two.encoding_length().unwrap();
        assert_eq!(length, 4);
    }

    #[test]
    fn test_box_encode_into() {
        let test = Box::new(42);
        let mut vec = vec![12];
        test.encode_into(&mut vec).unwrap();
        assert_eq!(*test, 42);
    }

    #[test]
    fn test_box_decode() {
        let bytes = vec![1];
        let test = Box::new(bytes.as_slice());
        let decoded_value: Box<bool> = Decode::decode(test).unwrap();
        assert_eq!(*decoded_value, true);
    }

    #[test]
    fn test_box_decode_into() {
        let mut test = Box::new(false);
        let bytes = vec![1];
        test.decode_into(bytes.as_slice()).unwrap();
        assert_eq!(*test, true);
    }

    #[test]
    fn test_slice_encode_into() {
        let vec = vec![1, 2, 1];
        let slice = &vec[0..3];
        let mut vec: Vec<u8> = vec![];
        slice.encode_into(&mut vec).unwrap();
        assert_eq!(vec, vec![0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1]);
    }

    #[test]
    fn test_slice_encoding_length() {
        let vec = vec![1, 2, 1];
        let slice = &vec[0..3];
        let size = slice.encoding_length().unwrap();
        assert_eq!(size, 12);
    }

    #[test]
    fn test_unit_encoding_length() {
        let unit = ();
        let length = unit.encoding_length().unwrap();
        assert!(length == 0);
    }

    #[test]
    fn test_phantom_data_encoding_length() {
        use std::marker::PhantomData;
        let pd: PhantomData<u8> = PhantomData;
        let length = pd.encoding_length().unwrap();
        assert_eq!(length, 0);
    }
}
