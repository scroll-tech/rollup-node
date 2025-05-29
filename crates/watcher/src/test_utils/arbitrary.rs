pub(crate) use arbitrary::Arbitrary;

/// Returns an arbitrary instance of the passed type.
#[macro_export]
macro_rules! random {
    ($typ: ty) => {{
        let mut bytes = Box::new([0u8; size_of::<$typ>()]);
        let mut rng = ::rand::rng();
        ::rand::RngCore::fill_bytes(&mut rng, bytes.as_mut_slice());
        let mut u = ::arbitrary::Unstructured::new(bytes.as_slice());
        <$typ>::arbitrary(&mut u).unwrap()
    }};
}
