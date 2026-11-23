//! The machine families linked into this program, selected by cargo features.

use plugin_api::MachinePlugin;

#[cfg(all(
    feature = "variant-arcade",
    any(
        feature = "sinclair",
        feature = "amstrad",
        feature = "commodore",
        feature = "atari",
        feature = "sega",
        feature = "nec"
    )
))]
compile_error!(
    "rcda is the arcade-only program: build it with --no-default-features --features variant-arcade"
);
#[cfg(all(
    feature = "variant-console",
    any(
        feature = "sinclair",
        feature = "amstrad",
        feature = "commodore",
        feature = "atari"
    )
))]
compile_error!(
    "rcdc is the console-only program: build it with --no-default-features --features variant-console"
);
#[cfg(all(
    feature = "variant-home",
    any(feature = "sega", feature = "snk", feature = "nec")
))]
compile_error!(
    "rcdh is the home-computer-only program: build it with --no-default-features --features variant-home"
);

/// Every machine family this build contains.
#[allow(unused_mut, clippy::vec_init_then_push)]
pub(crate) fn all() -> Vec<&'static dyn MachinePlugin> {
    let mut plugins: Vec<&'static dyn MachinePlugin> = Vec::new();
    #[cfg(feature = "sinclair")]
    {
        plugins.push(zx81_machine_plugin_impl::plugin());
        plugins.push(zx_spectrum_machine_plugin_impl::plugin());
    }
    #[cfg(feature = "amstrad")]
    plugins.push(cpc_machine_plugin_impl::plugin());
    #[cfg(feature = "commodore")]
    {
        plugins.push(c64_machine_plugin_impl::plugin());
        plugins.push(c128_machine_plugin_impl::plugin());
        plugins.push(vic20_machine_plugin_impl::plugin());
    }
    #[cfg(feature = "atari")]
    {
        plugins.push(atari_st_machine_plugin_impl::plugin());
        plugins.push(atari_8bit_machine_plugin_impl::plugin());
    }
    #[cfg(feature = "snk")]
    plugins.push(neogeo_machine_plugin_impl::plugin());
    #[cfg(feature = "sega")]
    plugins.push(sega_machine_plugin_impl::plugin());
    #[cfg(feature = "nec")]
    plugins.push(pcengine_machine_plugin_impl::plugin());
    plugins
}

/// The `firmware.toml` of every family in this build: which ROM file fills which machine option, and where
/// `--firmware auto` finds it.
pub(crate) fn firmware_manifests() -> Vec<(&'static str, &'static str)> {
    let mut manifests: Vec<(&'static str, &'static str)> = Vec::new();
    #[cfg(feature = "sinclair")]
    manifests.push((
        "machines/sinclair/firmware.toml",
        include_str!("../../../machines/sinclair/firmware.toml"),
    ));
    #[cfg(feature = "amstrad")]
    manifests.push((
        "machines/amstrad/firmware.toml",
        include_str!("../../../machines/amstrad/firmware.toml"),
    ));
    #[cfg(feature = "commodore")]
    manifests.push((
        "machines/commodore/firmware.toml",
        include_str!("../../../machines/commodore/firmware.toml"),
    ));
    #[cfg(feature = "atari")]
    manifests.push((
        "machines/atari/firmware.toml",
        include_str!("../../../machines/atari/firmware.toml"),
    ));
    #[cfg(feature = "snk")]
    manifests.push((
        "machines/snk/firmware.toml",
        include_str!("../../../machines/snk/firmware.toml"),
    ));
    #[cfg(feature = "sega")]
    manifests.push((
        "machines/sega/firmware.toml",
        include_str!("../../../machines/sega/firmware.toml"),
    ));
    #[cfg(feature = "nec")]
    manifests.push((
        "machines/nec/firmware.toml",
        include_str!("../../../machines/nec/firmware.toml"),
    ));
    manifests
}
