//! `--at FRAME:ACTION[=ARG]`: the hotkey and file-dialog actions for a `--headless --frames` run. Nothing is asked: the file an
//! action needs is named on the command line, and the action itself is the emu thread's own (`HostControl::apply`), so a reset, a
//! quick save or a disk swap behaves exactly as it does in the window. Names are the hotkey action names (plus `key_down`/`key_up`
//! and `mouse_move`/`mouse_down`/`mouse_up`).

use std::sync::mpsc;

use crate::emu_thread::{EmuCommand, Export, HostControl, MediaOp, MediaSettings, Messages};
use crate::plugin_bridge::HostedMachine;

/// What one `--at` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    Reset,
    Freeze,
    /// `key_down=NAME` / `key_up=NAME`: press or release a machine key by the name `--list-keys` prints (a joystick direction,
    /// a key with no character to `--type`), so a run can hold it for as many frames as it wants.
    KeyDown(String),
    KeyUp(String),
    /// `mouse_move=DX,DY`: host pixels, right and down positive; sent as relative motion events of at most 127 each.
    MouseMove(i32, i32),
    /// `mouse_down=left|right|middle` / `mouse_up=...`.
    MouseButton(MouseKey, bool),
    /// `screenshot` -> `screenshots/<machine>-<time>.png`, `screenshot=FILE` -> that file.
    Screenshot(Option<String>),
    QuickSave,
    QuickLoad,
    StateSlot(i8),
    LoadTape(String),
    LoadDisk(String),
    EjectTape,
    EjectDisk,
    LoadDiskB(String),
    EjectDiskB,
    SaveDiskB(String),
    LoadSnapshot(String),
    SaveSnapshot(String),
    LoadState(String),
    SaveState(String),
    SaveTape(String),
    SaveDisk(String),
}

/// A mouse button for `mouse_down`/`mouse_up`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseKey {
    Left,
    Right,
    Middle,
}

/// `(dx, dy)` as equal relative steps that each fit an `i8` (-127..=127) and sum exactly to the move.
pub(crate) fn motion_steps(dx: i32, dy: i32) -> Vec<(i8, i8)> {
    let n = dx.unsigned_abs().max(dy.unsigned_abs()).div_ceil(127) as i64;
    (0..n)
        .map(|i| {
            let part = |d: i32| (d as i64 * (i + 1) / n - d as i64 * i / n) as i8;
            (part(dx), part(dy))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scripted {
    pub(crate) frame: u32,
    pub(crate) action: Action,
}

/// Parses one `--at` value: `FRAME:ACTION` or `FRAME:ACTION=ARG` (the first `:` ends the frame, so ARG may be `C:\x.dsk`).
pub(crate) fn parse(text: &str) -> Result<Scripted, String> {
    let (frame, rest) = text
        .split_once(':')
        .ok_or_else(|| format!("--at {text}: expected FRAME:ACTION[=ARG]"))?;
    let frame: u32 = frame
        .trim()
        .parse()
        .map_err(|_| format!("--at {text}: {frame:?} is not a frame number"))?;
    let (name, arg) = match rest.split_once('=') {
        Some((name, arg)) => (name.trim(), Some(arg.trim().to_string())),
        None => (rest.trim(), None),
    };
    let name = name.to_ascii_lowercase().replace('-', "_");
    let need = |arg: Option<String>| {
        arg.filter(|a| !a.is_empty())
            .ok_or_else(|| format!("--at {text}: {name} needs a file: {name}=FILE"))
    };
    let need_name = |arg: Option<String>| {
        arg.filter(|a| !a.is_empty())
            .ok_or_else(|| format!("--at {text}: {name} needs a machine key name: {name}=Joy1Up"))
    };
    let none = |arg: &Option<String>| match arg {
        Some(_) => Err(format!("--at {text}: {name} takes no argument")),
        None => Ok(()),
    };
    let action = match name.as_str() {
        "reset" => none(&arg).map(|()| Action::Reset)?,
        "freeze" => none(&arg).map(|()| Action::Freeze)?,
        "key_down" => Action::KeyDown(need_name(arg)?),
        "key_up" => Action::KeyUp(need_name(arg)?),
        "mouse_move" => {
            let arg = arg.unwrap_or_default();
            let parsed = arg
                .split_once(',')
                .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)));
            let (dx, dy) = parsed.ok_or_else(|| {
                format!("--at {text}: mouse_move needs DX,DY in host pixels (mouse_move=-200,30)")
            })?;
            Action::MouseMove(dx, dy)
        }
        "mouse_down" | "mouse_up" => {
            let button = match arg.unwrap_or_default().to_ascii_lowercase().as_str() {
                "left" => MouseKey::Left,
                "right" => MouseKey::Right,
                "middle" => MouseKey::Middle,
                other => {
                    return Err(format!(
                        "--at {text}: {name} needs a button: left, right or middle (not {other:?})"
                    ));
                }
            };
            Action::MouseButton(button, name == "mouse_down")
        }
        "screenshot" => Action::Screenshot(arg.filter(|a| !a.is_empty())),
        "quick_save" => none(&arg).map(|()| Action::QuickSave)?,
        "quick_load" => none(&arg).map(|()| Action::QuickLoad)?,
        "state_slot_next" => none(&arg).map(|()| Action::StateSlot(1))?,
        "state_slot_prev" => none(&arg).map(|()| Action::StateSlot(-1))?,
        "eject_tape" => none(&arg).map(|()| Action::EjectTape)?,
        "eject_disk" => none(&arg).map(|()| Action::EjectDisk)?,
        "eject_disk_b" => none(&arg).map(|()| Action::EjectDiskB)?,
        "load_disk_b" => Action::LoadDiskB(need(arg)?),
        "save_disk_b" => Action::SaveDiskB(need(arg)?),
        "load_tape" => Action::LoadTape(need(arg)?),
        "load_disk" => Action::LoadDisk(need(arg)?),
        "load_snapshot" => Action::LoadSnapshot(need(arg)?),
        "save_snapshot" => Action::SaveSnapshot(need(arg)?),
        "load_state" => Action::LoadState(need(arg)?),
        "save_state" => Action::SaveState(need(arg)?),
        "save_tape" => Action::SaveTape(need(arg)?),
        "save_disk" => Action::SaveDisk(need(arg)?),
        "pause" | "fast_forward" | "mute" | "volume_up" | "volume_down" | "overlay" | "version"
        | "fullscreen" | "quit" | "paste" | "shader_next" | "shader_prev" => {
            return Err(format!(
                "--at {text}: {name} means nothing in a headless run (no window, no sound)"
            ));
        }
        _ => {
            return Err(format!(
                "--at {text}: unknown action {name:?} (reset, freeze, screenshot[=FILE], key_down=K, key_up=K, mouse_move=DX,DY, \
                 mouse_down=B, mouse_up=B, quick_save, quick_load, \
                 state_slot_next, state_slot_prev, load_tape=F, load_disk=F, eject_tape, eject_disk, load_disk_b=F, eject_disk_b, save_disk_b=F, load_snapshot=F, \
                 save_snapshot=F, load_state=F, save_state=F, save_tape=F, save_disk=F)"
            ));
        }
    };
    Ok(Scripted { frame, action })
}

/// Runs the actions of a headless run in frame order.
pub(crate) struct Runner {
    control: HostControl,
    messages: Messages,
    pending: std::collections::VecDeque<Scripted>,
    /// An action failed (its message said so): the run exits with an error.
    pub(crate) failed: bool,
}

impl Runner {
    pub(crate) fn new(machine_id: &str, media: MediaSettings, mut actions: Vec<Scripted>) -> Self {
        actions.sort_by_key(|a| a.frame); // stable: same-frame actions keep their command-line order
        let messages: Messages = Messages::default();
        Runner {
            control: HostControl::new(0, machine_id, messages.clone(), media),
            messages,
            pending: actions.into(),
            failed: false,
        }
    }

    /// Runs everything due at or before `frame` (before that frame executes).
    pub(crate) fn run_due(&mut self, frame: u32, machine: &mut HostedMachine) {
        while self.pending.front().is_some_and(|a| a.frame <= frame) {
            let scripted = self.pending.pop_front().expect("checked");
            self.run_one(scripted, machine);
        }
    }

    fn run_one(&mut self, scripted: Scripted, machine: &mut HostedMachine) {
        let Scripted { frame, action } = scripted;
        let result = self.perform(&action, machine);
        for message in self.messages.lock().unwrap().drain(..) {
            if is_failure(&message) {
                self.failed = true;
                eprintln!("frame {frame}: {message}");
            } else {
                println!("frame {frame}: {message}");
            }
        }
        match result {
            Ok(Some(note)) => println!("frame {frame}: {note}"),
            Ok(None) => {}
            Err(e) => {
                self.failed = true;
                eprintln!("frame {frame}: {e}");
            }
        }
    }

    fn perform(
        &mut self,
        action: &Action,
        machine: &mut HostedMachine,
    ) -> Result<Option<String>, String> {
        let command = |this: &mut Self, c: EmuCommand, machine: &mut HostedMachine| {
            this.control.apply(c, machine);
            Ok(None)
        };
        match action {
            Action::Reset => command(self, EmuCommand::Reset, machine),
            Action::Freeze => command(self, EmuCommand::Freeze, machine),
            Action::QuickSave => command(self, EmuCommand::QuickSave, machine),
            Action::QuickLoad => command(self, EmuCommand::QuickLoad, machine),
            Action::StateSlot(delta) => command(self, EmuCommand::StateSlot(*delta), machine),
            Action::EjectTape => command(self, EmuCommand::Media(MediaOp::EjectTape), machine),
            Action::EjectDisk => command(self, EmuCommand::Media(MediaOp::EjectDisk), machine),
            Action::EjectDiskB => command(self, EmuCommand::Media(MediaOp::EjectDiskB), machine),
            Action::LoadDiskB(path) => {
                let (extension, bytes) = read(path)?;
                let op = MediaOp::AttachDiskB {
                    extension,
                    bytes,
                    path: path.into(),
                };
                command(self, EmuCommand::Media(op), machine)
            }
            Action::SaveDiskB(path) => {
                let note = self.save(Export::DiskB, path, machine)?;
                self.control
                    .apply(EmuCommand::Media(MediaOp::DiskBSaved), machine);
                Ok(note)
            }
            Action::KeyDown(key) | Action::KeyUp(key) => {
                machine
                    .try_key_event(key, matches!(action, Action::KeyDown(_)))
                    .map_err(|e| format!("key {key}: {e}"))?;
                Ok(None)
            }
            Action::MouseMove(dx, dy) => {
                for (dx, dy) in motion_steps(*dx, *dy) {
                    machine
                        .try_mouse_event(crate::emu_thread::MouseEvent::Motion { dx, dy })
                        .map_err(|e| format!("mouse_move: {e}"))?;
                }
                Ok(None)
            }
            Action::MouseButton(button, pressed) => {
                let button = match button {
                    MouseKey::Left => bevy::input::mouse::MouseButton::Left,
                    MouseKey::Right => bevy::input::mouse::MouseButton::Right,
                    MouseKey::Middle => bevy::input::mouse::MouseButton::Middle,
                };
                machine
                    .try_mouse_event(crate::emu_thread::MouseEvent::Button {
                        button,
                        pressed: *pressed,
                    })
                    .map_err(|e| format!("mouse button: {e}"))?;
                Ok(None)
            }
            Action::Screenshot(None) => command(self, EmuCommand::Screenshot, machine),
            Action::Screenshot(Some(path)) => {
                let (w, h, rgb) = machine
                    .render_frame()
                    .ok_or("screenshot: the machine has no picture")?;
                crate::write_png(path, w, h, &rgb)
                    .map_err(|e| format!("screenshot failed: {e}"))?;
                Ok(Some(format!("Screenshot saved: {path}")))
            }
            Action::LoadTape(path) => {
                let (extension, bytes) = read(path)?;
                command(
                    self,
                    EmuCommand::Media(MediaOp::AttachTape { extension, bytes }),
                    machine,
                )
            }
            Action::LoadDisk(path) => {
                let (extension, bytes) = read(path)?;
                let op = MediaOp::AttachDisk {
                    extension,
                    bytes,
                    path: path.into(),
                };
                command(self, EmuCommand::Media(op), machine)
            }
            Action::LoadSnapshot(path) => {
                let (extension, bytes) = read(path)?;
                command(
                    self,
                    EmuCommand::Media(MediaOp::LoadSnapshot { extension, bytes }),
                    machine,
                )
            }
            Action::LoadState(path) => {
                let (_, bytes) = read(path)?;
                command(
                    self,
                    EmuCommand::Media(MediaOp::LoadState { bytes }),
                    machine,
                )
            }
            Action::SaveState(path) => self.save(Export::State, path, machine),
            Action::SaveSnapshot(path) => {
                let extension = crate::inputs::extension(path);
                self.save(Export::Snapshot { extension }, path, machine)
            }
            Action::SaveTape(path) => {
                let format = crate::inputs::extension(path);
                self.save(Export::Tape { format }, path, machine)
            }
            Action::SaveDisk(path) => self.save(Export::Disk, path, machine),
        }
    }

    fn save(
        &mut self,
        what: Export,
        path: &str,
        machine: &mut HostedMachine,
    ) -> Result<Option<String>, String> {
        let (tx, rx) = mpsc::channel();
        self.control.apply(EmuCommand::Export(what, tx), machine);
        match rx.recv().map_err(|e| e.to_string())?? {
            Some(bytes) => {
                std::fs::write(path, &bytes).map_err(|e| format!("could not write {path}: {e}"))?;
                Ok(Some(format!("Saved {} bytes to {path}", bytes.len())))
            }
            None => Err(format!("nothing to save to {path}")),
        }
    }

    /// Actions scheduled for a frame the run never reached.
    #[cfg(test)]
    pub(crate) fn unreached(&self) -> Vec<u32> {
        self.pending.iter().map(|a| a.frame).collect()
    }
}

/// The file's extension (by what it really is, a zip entry included) and bytes.
fn read(path: &str) -> Result<(String, Vec<u8>), String> {
    let loaded = crate::inputs::load(path)?;
    Ok((crate::inputs::extension(path), loaded.bytes.clone()))
}

/// The toasts the actions post when something went wrong.
fn is_failure(message: &str) -> bool {
    message.contains("failed")
        || message.starts_with("No saved state")
        || message.starts_with("Freeze:")
        || message.starts_with("Drive B has unsaved changes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_values_parse() {
        assert_eq!(
            parse("100:reset").unwrap(),
            Scripted {
                frame: 100,
                action: Action::Reset
            }
        );
        assert_eq!(
            parse("5:load_disk=C:\\games\\a.dsk").unwrap().action,
            Action::LoadDisk("C:\\games\\a.dsk".into())
        );
        assert_eq!(
            parse("7:State-Slot-Next").unwrap().action,
            Action::StateSlot(1)
        );
        assert_eq!(
            parse("2:load_disk_b=b.dsk").unwrap().action,
            Action::LoadDiskB("b.dsk".into())
        );
        assert_eq!(parse("2:eject_disk_b").unwrap().action, Action::EjectDiskB);
        assert_eq!(
            parse("2:save_disk_b=o.dsk").unwrap().action,
            Action::SaveDiskB("o.dsk".into())
        );
        assert_eq!(
            parse("9:screenshot").unwrap().action,
            Action::Screenshot(None)
        );
        assert_eq!(
            parse("9:screenshot=out.png").unwrap().action,
            Action::Screenshot(Some("out.png".into()))
        );
    }

    #[test]
    fn bad_at_values_say_why() {
        assert!(parse("reset").unwrap_err().contains("FRAME:ACTION"));
        assert!(parse("x:reset").unwrap_err().contains("frame number"));
        assert!(parse("1:load_tape").unwrap_err().contains("needs a file"));
        assert!(parse("1:key_down").unwrap_err().contains("key name"));
        assert!(parse("1:reset=now").unwrap_err().contains("no argument"));
        assert!(parse("1:pause").unwrap_err().contains("headless"));
        assert!(parse("1:jump").unwrap_err().contains("unknown action"));
    }

    #[test]
    fn drive_b_actions_load_save_and_eject_through_files() {
        let machine_id = "stub-machine";
        let mut machine =
            crate::plugin_bridge::create_hosted_machine(plugin_api::MachineCreateRequest {
                machine_id: machine_id.to_string(),
                region: Some("pal".to_string()),
                roms: Vec::new(),
                options: Vec::new(),
            })
            .expect("the bundled stub machine");
        let dir = std::env::temp_dir().join(format!("rc-scripted-b-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let disk = dir.join("b.dsk");
        let copy = dir.join("copy.dsk");
        std::fs::write(&disk, dsk::Disk::new(1).to_bytes()).unwrap();
        let actions = vec![
            parse(&format!("0:load_disk_b={}", disk.display())).unwrap(),
            parse(&format!("1:save_disk_b={}", copy.display())).unwrap(),
            parse("2:eject_disk_b").unwrap(),
            parse(&format!("3:save_disk_b={}", copy.display())).unwrap(),
        ];
        let mut runner = Runner::new(machine_id, MediaSettings::default(), actions);
        for frame in 0..3 {
            runner.run_due(frame, &mut machine);
        }
        assert!(!runner.failed);
        assert!(copy.exists(), "save_disk_b wrote a copy of drive B");
        assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&disk).unwrap());
        // Nothing left in drive B to save: that is a failed action, not a silent no-op.
        runner.run_due(3, &mut machine);
        assert!(runner.failed);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn actions_run_in_frame_order_on_a_machine() {
        let machine_id = "stub-machine";
        let mut machine =
            crate::plugin_bridge::create_hosted_machine(plugin_api::MachineCreateRequest {
                machine_id: machine_id.to_string(),
                region: Some("pal".to_string()),
                roms: Vec::new(),
                options: Vec::new(),
            })
            .expect("the bundled stub machine");
        let dir = std::env::temp_dir().join(format!("rc-scripted-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("a.state");
        let state_text = state.to_string_lossy().into_owned();
        let actions = vec![
            parse(&format!("3:load_state={state_text}")).unwrap(),
            parse(&format!("1:save_state={state_text}")).unwrap(),
            parse("2:reset").unwrap(),
            parse("9:reset").unwrap(),
        ];
        let mut runner = Runner::new(machine_id, MediaSettings::default(), actions);
        for frame in 0..4 {
            runner.run_due(frame, &mut machine);
        }
        assert!(!runner.failed);
        assert!(
            state.exists(),
            "save_state wrote the file before load_state read it"
        );
        assert_eq!(runner.unreached(), [9]);

        let mut runner = Runner::new(
            machine_id,
            MediaSettings::default(),
            vec![parse("0:load_state=/no/such/file").unwrap()],
        );
        runner.run_due(0, &mut machine);
        assert!(runner.failed, "a missing file fails the run");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parse_mouse_actions() {
        assert_eq!(
            parse("5:mouse_move=-200,30").unwrap().action,
            Action::MouseMove(-200, 30)
        );
        assert_eq!(
            parse("5:mouse_down=left").unwrap().action,
            Action::MouseButton(MouseKey::Left, true)
        );
        assert_eq!(
            parse("5:mouse_up=Right").unwrap().action,
            Action::MouseButton(MouseKey::Right, false)
        );
        assert!(parse("5:mouse_move=3").is_err());
        assert!(parse("5:mouse_move=a,b").is_err());
    }

    #[test]
    fn unknown_button_is_an_error() {
        let e = parse("5:mouse_down=thumb").unwrap_err();
        assert!(
            e.contains("left") && e.contains("right") && e.contains("middle"),
            "{e}"
        );
    }

    #[test]
    fn a_large_move_is_split_into_steps_that_sum_to_it() {
        for (dx, dy) in [(300, -200), (-1000, 7), (0, 0), (127, -128), (-129, 128)] {
            let steps = motion_steps(dx, dy);
            assert_eq!(steps.iter().map(|s| s.0 as i32).sum::<i32>(), dx);
            assert_eq!(steps.iter().map(|s| s.1 as i32).sum::<i32>(), dy);
            assert!(!steps.is_empty() || (dx, dy) == (0, 0));
        }
        assert_eq!(motion_steps(300, -200).len(), 3);
    }

    /// The stub machine adds the motion it receives to RAM `$FF00`/`$FF02` (little-endian) and keeps the buttons at
    /// `$FF04` (bit 0 left, 1 right, 2 middle).
    #[test]
    fn mouse_actions_reach_the_machine() {
        let machine_id = "stub-machine";
        let mut machine =
            crate::plugin_bridge::create_hosted_machine(plugin_api::MachineCreateRequest {
                machine_id: machine_id.to_string(),
                region: Some("pal".to_string()),
                roms: Vec::new(),
                options: Vec::new(),
            })
            .expect("the bundled stub machine");
        let actions = vec![
            parse("0:mouse_move=300,-200").unwrap(),
            parse("0:mouse_down=left").unwrap(),
            parse("1:mouse_down=right").unwrap(),
            parse("1:mouse_up=left").unwrap(),
        ];
        let mut runner = Runner::new(machine_id, MediaSettings::default(), actions);
        runner.run_due(0, &mut machine);
        runner.run_due(1, &mut machine);
        assert!(!runner.failed);
        let word =
            |a: u64| i16::from_le_bytes([machine.read_memory(a), machine.read_memory(a + 1)]);
        assert_eq!((word(0xFF00), word(0xFF02)), (300, -200));
        assert_eq!(machine.read_memory(0xFF04), 0b010);
    }
}
