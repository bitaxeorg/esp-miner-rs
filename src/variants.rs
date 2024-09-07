#[cfg(any(
    all(feature = "bitaxe-max", feature = "bitaxe-ultra"),
    not(any(feature = "bitaxe-max", feature = "bitaxe-ultra"))
))]
compile_error!(
    "You must activate exactly one of the following variant features:
    bitaxe-max,
    bitaxe-ultra,
    "
);
