//! The monochrome monitor: EmuTOS detects it (GPIP I7), switches to 640 x 400 and shows its desktop.

mod common;
use common::{config, emutos};

use atari_st::{Accuracy, AtariSt, RamSize};
use machine::Machine;

#[test]
fn emutos_mono_desktop_is_reached() {
    let Some(rom) = emutos() else {
        return;
    };
    for accuracy in [Accuracy::Fast, Accuracy::Accurate] {
        let mut cfg = config(rom.clone(), RamSize::M1);
        cfg.monitor = st_shifter::Monitor::Mono;
        cfg.accuracy = accuracy;
        let mut m = AtariSt::new(cfg).unwrap();
        while m.io_stop().is_none() && m.vbl_count() < 1500 {
            m.step();
        }
        assert!(m.io_stop().is_none(), "{accuracy:?}: {:?}", m.io_stop());
        let (w, h, rgb) = m.render_frame().unwrap();
        assert_eq!((w, h), (640, 400));
        let px = |x: u32, y: u32| rgb[((y * w + x) * 3) as usize];
        // The menu bar: dark text on a light bar, so both shades are in its rows.
        let bar: std::collections::HashSet<u8> = (0..18)
            .flat_map(|y| (0..640).map(move |x| (x, y)))
            .map(|(x, y)| px(x, y))
            .collect();
        assert!(
            bar.contains(&0) && bar.contains(&255),
            "{accuracy:?}: {bar:?}"
        );
        // The desktop below the bar is not black.
        let desk = (0..640).filter(|&x| px(x, 200) == 255).count();
        assert!(
            desk > 100,
            "{accuracy:?}: {desk} light pixels on a desktop row"
        );
    }
}
