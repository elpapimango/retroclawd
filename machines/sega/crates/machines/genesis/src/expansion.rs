//! Hardware that hangs on the cartridge port and takes over part of the 68000's address space: the Mega-CD. The bus offers every
//! access in `$000000-$3FFFFF` and `$A12000-$A120FF` to it first (when one is attached); an access it does not answer stops the
//! machine with the usual report.

use mc68000::Size;
use std::any::Any;

pub trait Expansion: Send {
    /// Whether the expansion answers (or stops on) this 24-bit address.
    fn claims(&self, addr: u32) -> bool {
        addr < 0x40_0000 || (0xA1_2000..=0xA1_20FF).contains(&addr)
    }

    /// A read at master clock `now`. `None`: no fact covers it.
    fn read(&mut self, addr: u32, size: Size, now: u64) -> Option<u32>;

    /// A write at master clock `now`. `false`: no fact covers it.
    fn write(&mut self, addr: u32, size: Size, value: u32, now: u64) -> bool;

    /// A word a VDP DMA reads from the expansion's part of the map. `None`: not readable by DMA.
    fn dma_read_word(&mut self, addr: u32) -> Option<u16>;

    /// A byte as the debugger sees it: no side effects.
    fn peek(&self, addr: u32) -> u8;

    /// Runs the expansion's own processors up to master clock `now`.
    fn advance(&mut self, now: u64);

    /// Adds the expansion's own sound to `out` (interleaved left and right samples, one pair every `frame_clocks` master clocks).
    fn mix_audio(&mut self, _out: &mut [f32], _frame_clocks: u64) {}

    /// The report of a stop inside the expansion (its own CPU met an unknown address), once it happened.
    fn stop_report(&self) -> Option<String>;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
