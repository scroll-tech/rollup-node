/// Copies the provided slice into $ty using $ty::from_be_bytes and advances the buffer.
#[macro_export]
macro_rules! from_be_bytes_slice_and_advance_buf {
    ($ty: ty, $slice: expr) => {{
        $crate::from_be_bytes_slice_and_advance_buf!($ty, ::std::mem::size_of::<$ty>(), $slice)
    }};
    ($ty:ty, $size: expr, $slice: expr) => {{
        let mut arr = [0u8; ::std::mem::size_of::<$ty>()];
        let size = $size;
        let size_of = ::std::mem::size_of::<$ty>();
        arr[size_of - size..].copy_from_slice(&$slice[..size]);
        ::alloy_primitives::bytes::Buf::advance($slice, size);
        <$ty>::from_be_bytes(arr)
    }};
}

/// Calls `from_slice` on the provided type using the passed in buffer and advances it.
#[macro_export]
macro_rules! from_slice_and_advance_buf {
    ($ty:ty, $slice: expr) => {{
        let size_of = ::std::mem::size_of::<$ty>();
        let t = <$ty>::from_slice(&$slice[..size_of]);
        ::alloy_primitives::bytes::Buf::advance($slice, size_of);
        t
    }};
}

/// Check the buffer input to have the required length. Returns an Eof error otherwise.
#[macro_export]
macro_rules! check_buf_len {
    ($buf: expr, $len: expr) => {{
        if $buf.len() < $len {
            return Err($crate::error::DecodingError::Eof)
        }
    }};
}
