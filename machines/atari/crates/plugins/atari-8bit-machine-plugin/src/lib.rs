//! Atari 8-bit Plugin. Only the C ABI boundary uses unsafe code.
use atari_8bit::{Atari800, Profile, Roms, Timing};
use machine::Machine;
use plugin_api::{
    ABI_MAJOR_V1, ABI_MINOR_V1, AddressSpaceInfo, BitmapExplorerDefaults, BitmapPaletteColor,
    CartridgeStatus, ChipSnapshot, CpuFamily, CpuInfo, DebugCommand, DebugResponse, DebugValue,
    DisassemblyLine, DiskStatus, ExecutionMode, ListProfilesRequest, ListProfilesResponse,
    MACHINE_PROFILE_CONTRACT_VERSION, MachineCapability, MachineCommand, MachineCreateRequest,
    MachineCreateResponse, MachineInstanceCallRequest, MachineInstanceCallResponse, MachineKeyInfo,
    MachineMetadata, MachineOptionInfo, MachineOptionKind, MachinePluginVTableV1, MachineProfile,
    MachineResponse, MediaKind, MediaSlot, MediaSnapshot, NamedValue, PluginDescriptorV1,
    PluginKind, REQUIRED_MACHINE_DEBUG_CAPABILITIES, RcBuffer, RcCallResult, RcSlice, RcStatus,
    RegisterField, RegisterSnapshot, StatusCode, StepResult, TapeStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};

const PROFILES: [Profile; 4] = [
    Profile::Atari800,
    Profile::Atari800Xl,
    Profile::Atari65Xe,
    Profile::Atari130Xe,
];
const COMBOS: &[(&str, &str)] = &[
    ("Exclamation", "Digit1"),
    ("DoubleQuote", "Digit2"),
    ("Hash", "Digit3"),
    ("Dollar", "Digit4"),
    ("Percent", "Digit5"),
    ("Ampersand", "Digit6"),
    ("Apostrophe", "Digit7"),
    ("At", "Digit8"),
    ("LeftParen", "Digit9"),
    ("RightParen", "Digit0"),
    ("Colon", "Semicolon"),
    ("Underscore", "Minus"),
    ("Pipe", "Equals"),
    ("Backslash", "Plus"),
    ("Caret", "Asterisk"),
    ("LeftBracket", "Comma"),
    ("RightBracket", "Period"),
    ("Question", "Slash"),
];
fn key_names(model: Profile) -> Vec<MachineKeyInfo> {
    let mut names = plugin_api::key_infos(
        atari_8bit::KEYS
            .iter()
            .map(|&(name, _)| name)
            .chain([
                "ShiftLeft",
                "ShiftRight",
                "Control",
                "Break",
                "Start",
                "Select",
                "Option",
            ])
            .chain(COMBOS.iter().map(|&(name, _)| name)),
    );
    if model.is_xl_xe() {
        names.extend(plugin_api::key_infos(["Help"]));
    }
    let joysticks: Vec<_> = (1..=if model.is_xl_xe() { 2 } else { 4 })
        .flat_map(|port| {
            ["Up", "Down", "Left", "Right", "Fire1"].map(move |key| format!("Joy{port}{key}"))
        })
        .collect();
    names.extend(plugin_api::key_infos(joysticks.iter().map(String::as_str)));
    for key in &mut names {
        key.aliases = match key.name.as_str() {
            "Return" => vec!["Enter".to_string()],
            "Escape" => vec!["Esc".to_string()],
            "Control" => vec!["ControlLeft".to_string(), "ControlRight".to_string()],
            _ => Vec::new(),
        };
    }
    plugin_api::set_key_chars(
        &mut names,
        &[
            ("Space", " "),
            ("Semicolon", ";"),
            ("Plus", "+"),
            ("Asterisk", "*"),
            ("Minus", "-"),
            ("Equals", "="),
            ("Comma", ","),
            ("Period", "."),
            ("Slash", "/"),
            ("LessThan", "<"),
            ("GreaterThan", ">"),
            ("Exclamation", "!"),
            ("DoubleQuote", "\""),
            ("Hash", "#"),
            ("Dollar", "$"),
            ("Percent", "%"),
            ("Ampersand", "&"),
            ("Apostrophe", "'"),
            ("At", "@"),
            ("LeftParen", "("),
            ("RightParen", ")"),
            ("Colon", ":"),
            ("Underscore", "_"),
            ("Pipe", "|"),
            ("Backslash", "\\"),
            ("Caret", "^"),
            ("LeftBracket", "["),
            ("RightBracket", "]"),
            ("Question", "?"),
        ],
    );
    names
}
fn profile(model: Profile) -> MachineProfile {
    let file = |key: &str, required: bool, description: &str| MachineOptionInfo {
        key: key.to_string(),
        flag: key.replace('_', "-"),
        short: None,
        kind: MachineOptionKind::File,
        default: None,
        required,
        description: description.to_string(),
    };
    MachineProfile {
        machine_id: model.id().to_string(),
        display_name: match model {
            Profile::Atari800 => "Atari 800",
            Profile::Atari800Xl => "Atari 800XL",
            Profile::Atari65Xe => "Atari 65XE",
            Profile::Atari130Xe => "Atari 130XE",
        }
        .to_string(),
        cpu_family: CpuFamily::Mos6502,
        default_region: Some("pal".to_string()),
        contract_version: MACHINE_PROFILE_CONTRACT_VERSION,
        debug_capabilities: REQUIRED_MACHINE_DEBUG_CAPABILITIES.to_vec(),
        capabilities: vec![
            MachineCapability::Video,
            MachineCapability::Audio,
            MachineCapability::SaveState,
            MachineCapability::Debug,
            MachineCapability::Disk,
            MachineCapability::Tape,
            MachineCapability::Cartridge,
        ],
        media_kinds: vec![
            MediaKind::Program,
            MediaKind::Disk,
            MediaKind::Tape,
            MediaKind::Cartridge,
        ],
        options: vec![
            file(
                "rom_os",
                true,
                if model.is_xl_xe() {
                    "XL/XE OS ROM, exactly 16384 bytes; explicit original or replacement ROM"
                } else {
                    "Atari 800 OS ROM, exactly 10240 bytes; explicit original or replacement ROM"
                },
            ),
            file("rom_basic", false, "Optional 8192-byte BASIC ROM cartridge"),
            ExecutionMode::option(
                "fast (default): instruction-batched accesses, not raster-exact; accurate: per-bus-cycle Chip clocks, DMA HALT, WSYNC and interrupt polling",
            ),
        ],
        key_names: key_names(model),
        cpus: vec![CpuInfo {
            id: "main".to_string(),
            name: "MOS 6502".to_string(),
            family: CpuFamily::Mos6502,
            address_space: "cpu".to_string(),
        }],
        axis_names: Vec::new(),
    }
}
struct Instance {
    machine: Atari800,
    held: BTreeSet<String>,
    breakpoints: BTreeSet<u64>,
    paused: bool,
    border: bool,
}
#[derive(Serialize, Deserialize)]
struct InputState {
    magic: [u8; 4],
    machine: Vec<u8>,
    held: BTreeSet<String>,
}
impl Instance {
    fn create(request: MachineCreateRequest) -> Result<Self, String> {
        let model = PROFILES
            .into_iter()
            .find(|model| model.id() == request.machine_id)
            .ok_or_else(|| format!("unknown Machine {:?}", request.machine_id))?;
        let pal = match request.region.as_deref() {
            None | Some("pal") => true,
            Some("ntsc") => false,
            Some(region) => {
                return Err(format!(
                    "unsupported Atari 800 Region {region:?}; expected pal or ntsc"
                ));
            }
        };
        let execution = ExecutionMode::from_options(&request.options)?.unwrap_or_default();
        let mut seen = BTreeSet::new();
        for option in &request.options {
            if option.key != "execution_mode" || !seen.insert(&option.key) {
                return Err(format!(
                    "unknown or duplicate Atari 800 Option {:?}",
                    option.key
                ));
            }
        }
        let mut roms = BTreeMap::new();
        for blob in request.roms {
            if !["rom_os", "rom_basic"].contains(&blob.key.as_str())
                || roms.insert(blob.key.clone(), blob.bytes).is_some()
            {
                return Err(format!("unknown or duplicate ROM {:?}", blob.key));
            }
        }
        Ok(Self {
            machine: Atari800::with_profile(
                model,
                Roms {
                    os: roms.remove("rom_os").ok_or("missing rom_os")?,
                    basic: roms.remove("rom_basic"),
                },
                pal,
                match execution {
                    ExecutionMode::Fast => Timing::Fast,
                    ExecutionMode::Accurate => Timing::Accurate,
                },
            )?,
            held: BTreeSet::new(),
            breakpoints: BTreeSet::new(),
            paused: false,
            border: true,
        })
    }
    fn step(&mut self) -> u32 {
        if self.paused || self.breakpoints.contains(&self.machine.pc()) {
            self.paused = true;
            4
        } else {
            self.machine.step()
        }
    }
    fn key(&mut self, name: &str, pressed: bool) -> Result<(), String> {
        let name = key_names(self.machine.profile())
            .into_iter()
            .find(|key| key.matches(name))
            .ok_or_else(|| format!("unknown Atari 800 Machine key {name:?}"))?
            .name;
        if pressed {
            self.held.insert(name);
        } else {
            self.held.remove(&name);
        }
        self.apply_keys()
    }
    fn apply_keys(&mut self) -> Result<(), String> {
        let mut keys = 0u64;
        let mut shift = self.held.contains("ShiftLeft") || self.held.contains("ShiftRight");
        for name in &self.held {
            if name == "Help" {
                keys |= 1u64 << 0x11;
            }
            let physical = if let Some(&(_, base)) = COMBOS.iter().find(|&&(key, _)| key == name) {
                shift = true;
                base
            } else {
                name
            };
            if let Some(&(_, code)) = atari_8bit::KEYS.iter().find(|&&(key, _)| key == physical) {
                keys |= 1u64 << code;
            }
        }
        self.machine.set_keyboard(
            keys,
            shift,
            self.held.contains("Control"),
            self.held.contains("Break"),
        );
        for (bit, name) in ["Start", "Select", "Option"].iter().enumerate() {
            self.machine
                .set_console(bit as u8, self.held.contains(*name))?;
        }
        for port in 1..=self.machine.joystick_count() {
            let directions =
                ["Up", "Down", "Left", "Right"]
                    .iter()
                    .enumerate()
                    .fold(0, |bits, (bit, name)| {
                        bits | (u8::from(self.held.contains(&format!("Joy{port}{name}"))) << bit)
                    });
            self.machine.set_joystick(
                port - 1,
                directions,
                self.held.contains(&format!("Joy{port}Fire1")),
            )?;
        }
        Ok(())
    }
    fn metadata(&self) -> MachineMetadata {
        let rate = self.machine.frame_rate();
        MachineMetadata {
            machine_id:self.machine.profile().id().to_string(),model:Some(self.machine.profile().id().to_string()),region:Some(if self.machine.pal() {"pal"} else {"ntsc"}.to_string()),
            frame_rate:(rate.num,rate.den),audio_channels:1,audio_sample_rate:machine::AUDIO_SAMPLE_RATE,
            bitmap_palette:gtia::Gtia::palette().into_iter().enumerate().map(|(i,rgb)| BitmapPaletteColor {name:format!("GTIA {i:02x}"),rgb}).collect(),
            bitmap_defaults:Some(BitmapExplorerDefaults {address:0x3000,bytes_width:40,bitplanes:1,zoom:2}),
            notes:vec![
                format!("{} KiB RAM; explicit OS and optional BASIC ROMs", self.machine.profile().ram_size()/1024),
                "ANTIC scrolling/DMA, GTIA P/M and modes 9/10/11; no composite artifacting or overlapping abnormal character-DMA model".to_string(),
                "Read/write ATR/XFD on D1:/D2: via PIA/POKEY serial pins; sector peripherals, not emulated 810 CPUs/FDCs or protected disks".to_string(),
                "Standard CAS playback and standard/XEGS CAR cartridges; no turbo cassette, recording or advanced mappers".to_string(),
                "POKEY digital timers/noise and linear mixer; analogue calibration unverified. CPU clock rounded down by 0.5 Hz".to_string(),
            ],
            ..MachineMetadata::default()
        }
    }
    fn memory(&self, address: u64, len: u32, space: Option<&str>) -> Result<Vec<u8>, String> {
        check_memory(address, len as usize, space)?;
        Ok((0..len)
            .map(|i| self.machine.read_memory((address + u64::from(i)) & 0xffff))
            .collect())
    }
    fn chips(&self) -> Vec<ChipSnapshot> {
        let mut chips: Vec<_> = ["antic", "gtia", "pokey", "pia"]
            .into_iter()
            .map(|name| ChipSnapshot {
                chip_id: name.to_string(),
                fields: self
                    .machine
                    .chip_registers(name)
                    .into_iter()
                    .enumerate()
                    .map(|(i, value)| NamedValue {
                        key: format!("r{i:02x}"),
                        value: DebugValue::U64(u64::from(value)),
                    })
                    .collect(),
                blobs: Vec::new(),
            })
            .collect();
        chips.push(ChipSnapshot {
            chip_id: "overlay".to_string(),
            fields: vec![
                NamedValue {
                    key: "memory_summary".to_string(),
                    value: DebugValue::Text(format!(
                        "{} KiB; {:?}; raster {:?}",
                        self.machine.profile().ram_size() / 1024,
                        self.machine.timing(),
                        self.machine.position()
                    )),
                },
                NamedValue {
                    key: "cycles".to_string(),
                    value: DebugValue::U64(self.machine.cycles()),
                },
            ],
            blobs: Vec::new(),
        });
        chips
    }
    fn media(&self) -> MediaSnapshot {
        MediaSnapshot {
            tape: self.machine.tape().map(|tape| TapeStatus {
                attached: true,
                playing: self.machine.tape_motor() && !tape.finished(),
                finished: tape.finished(),
                speed_multiplier: 1,
                position: Some(u64::from(tape.position().record)),
                total: None,
                motor_on: Some(self.machine.tape_motor()),
                ear_bit: Some(tape.input()),
                cycles_until_next_edge: None,
                spin_down_cycles: None,
            }),
            disks: (0..2)
                .map(|drive| DiskStatus {
                    attached: self.machine.disk_attached_in(drive),
                    image_id: None,
                    drive: Some(drive),
                    head: None,
                    track: None,
                    sector: Some(self.machine.disk_sector()),
                    controller_phase: Some(format!(
                        "SIO {}; {} reads; {}",
                        if self.machine.disk_dirty_in(drive) {
                            "modified"
                        } else {
                            "saved"
                        },
                        self.machine.disk_reads(),
                        if self.machine.disk_busy() {
                            "busy"
                        } else {
                            "idle"
                        }
                    )),
                })
                .collect(),
            cartridge: self.machine.cartridge().map(|cart| CartridgeStatus {
                attached: true,
                mapper: Some(format!("CAR type {}", cart.mapper().car_type())),
                active_banks: vec![NamedValue {
                    key: "bank".to_string(),
                    value: DebugValue::U64(u64::from(cart.bank())),
                }],
            }),
        }
    }
    fn debug(&mut self, mut command: DebugCommand) -> Result<DebugResponse, String> {
        while let DebugCommand::OnCpu {
            cpu,
            command: nested,
        } = command
        {
            if cpu != "main" {
                return Err(format!("unknown Atari 800 CPU {cpu:?}"));
            }
            command = *nested;
        }
        match command {
            DebugCommand::Pause => {
                self.paused = true;
                Ok(DebugResponse::Ack)
            }
            DebugCommand::Resume => {
                self.paused = false;
                Ok(DebugResponse::Ack)
            }
            DebugCommand::StepInstruction => {
                self.machine.step();
                Ok(DebugResponse::Ack)
            }
            DebugCommand::StepOver => {
                let pc = self.machine.pc();
                let stack = self.machine.cpu.sp;
                let call = self.machine.read_memory(pc) == 0x20;
                self.machine.step();
                if call {
                    for _ in 0..100_000 {
                        if self.machine.pc() == (pc + 3) & 0xffff && self.machine.cpu.sp == stack {
                            return Ok(DebugResponse::Ack);
                        }
                        if self.machine.halted() {
                            return Err("Atari 800 jammed during step-over".to_string());
                        }
                        if self.breakpoints.contains(&self.machine.pc()) {
                            self.paused = true;
                            return Ok(DebugResponse::Ack);
                        }
                        self.machine.step();
                    }
                    return Err("Atari 800 step-over exceeded 100000 instructions".to_string());
                }
                Ok(DebugResponse::Ack)
            }
            DebugCommand::SetBreakpoint { address } => {
                check_memory(address, 0, None)?;
                self.breakpoints.insert(address);
                Ok(DebugResponse::Ack)
            }
            DebugCommand::ClearBreakpoint { address } => {
                check_memory(address, 0, None)?;
                self.breakpoints.remove(&address);
                Ok(DebugResponse::Ack)
            }
            DebugCommand::ClearAllBreakpoints => {
                self.breakpoints.clear();
                Ok(DebugResponse::Ack)
            }
            DebugCommand::ListBreakpoints => Ok(DebugResponse::Breakpoints(
                self.breakpoints.iter().copied().collect(),
            )),
            DebugCommand::GetRegisters => Ok(DebugResponse::Registers(RegisterSnapshot {
                cpu_family: CpuFamily::Mos6502,
                registers: self
                    .machine
                    .registers()
                    .into_iter()
                    .map(|(name, value)| RegisterField {
                        bank: "main".to_string(),
                        name: name.to_string(),
                        value,
                        width_bits: if name == "PC" { 16 } else { 8 },
                    })
                    .collect(),
                flags: [
                    ("N", 7),
                    ("V", 6),
                    ("B", 4),
                    ("D", 3),
                    ("I", 2),
                    ("Z", 1),
                    ("C", 0),
                ]
                .into_iter()
                .map(|(name, bit)| NamedValue {
                    key: name.to_string(),
                    value: DebugValue::Bool(self.machine.cpu.p & (1 << bit) != 0),
                })
                .collect(),
            })),
            DebugCommand::SetRegister { name, value } => {
                self.machine.cpu.set_register(&name, value)?;
                Ok(DebugResponse::Ack)
            }
            DebugCommand::GetAddressSpaces => {
                Ok(DebugResponse::AddressSpaces(vec![AddressSpaceInfo {
                    name: "cpu".to_string(),
                    addr_width_bits: 16,
                    bank_context: Vec::new(),
                }]))
            }
            DebugCommand::ReadMemory {
                address,
                len,
                space,
            } => Ok(DebugResponse::Memory(self.memory(
                address,
                len,
                space.as_deref(),
            )?)),
            DebugCommand::WriteMemory {
                address,
                bytes,
                space,
            } => {
                check_memory(address, bytes.len(), space.as_deref())?;
                self.machine.load_program(address as u16, &bytes)?;
                Ok(DebugResponse::Ack)
            }
            DebugCommand::GetDisassembly { start, lines } => {
                let mut address = start.unwrap_or(self.machine.pc());
                check_memory(address, lines as usize, None)?;
                let mut output = Vec::new();
                for _ in 0..lines {
                    let raw = self.memory(address, 3, None)?;
                    let (text, len) = mos6502::disassemble::disassemble(&raw, address as u16);
                    output.push(DisassemblyLine {
                        address,
                        bytes: raw[..len].to_vec(),
                        text,
                    });
                    address = (address + len as u64) & 0xffff;
                }
                Ok(DebugResponse::Disassembly(output))
            }
            DebugCommand::GetMachineMetadata => Ok(DebugResponse::MachineMetadata(self.metadata())),
            DebugCommand::GetChipSnapshots => Ok(DebugResponse::ChipSnapshots(self.chips())),
            DebugCommand::GetMediaSnapshot => Ok(DebugResponse::MediaSnapshot(self.media())),
            DebugCommand::OnCpu { .. } => unreachable!("CPU selector removed"),
        }
    }
    fn command(&mut self, command: MachineCommand) -> Result<MachineResponse, String> {
        match command {
            MachineCommand::GetClockHz=>Ok(MachineResponse::ClockHz(self.machine.clock_hz())),
            MachineCommand::GetFrameHz=>Ok(MachineResponse::FrameHz(self.machine.frame_hz())),
            MachineCommand::RequestInterrupt=>Ok(MachineResponse::Ack),
            MachineCommand::Step=>{let cycles=self.step();Ok(MachineResponse::Step(StepResult{cycles,pc:self.machine.pc(),halted:self.machine.halted()}))}
            MachineCommand::GetPc=>Ok(MachineResponse::Pc(self.machine.pc())),
            MachineCommand::IsHalted=>Ok(MachineResponse::Halted(self.machine.halted())),
            MachineCommand::Reset=>{self.machine.reset();Ok(MachineResponse::Ack)}
            MachineCommand::ReadMemory{address,len,space}=>Ok(MachineResponse::Memory(self.memory(address,len,space.as_deref())?)),
            MachineCommand::WriteMemory{address,bytes,space}=>{
                check_memory(address,bytes.len(),space.as_deref())?;
                Ok(MachineResponse::BytesWritten(self.machine.load(address,&bytes) as u32))
            }
            MachineCommand::LoadBytes{address,bytes}=>{
                check_memory(address,bytes.len(),None)?;
                Ok(MachineResponse::BytesWritten(self.machine.load(address,&bytes) as u32))
            }
            MachineCommand::SetPc{address}=>{check_memory(address,0,None)?;self.machine.set_pc(address);Ok(MachineResponse::Ack)}
            MachineCommand::AttachMedia{slot:MediaSlot::Other(slot),kind:MediaKind::Program,name,bytes,options} if slot=="program" && name.as_deref().is_some_and(|name|name.to_ascii_lowercase().ends_with(".xex"))=>{
                if !options.is_empty() {return Err("Atari XEX uses its own segment addresses and accepts no org Option".to_string());}
                self.machine.load_xex(&bytes)?;
                Ok(MachineResponse::Ack)
            }
            MachineCommand::AttachMedia{slot:MediaSlot::Other(slot),kind:MediaKind::Program,bytes,options,..} if slot=="program"=>{
                let address=match options.as_slice(){
                    [NamedValue{key,value:DebugValue::U64(address)}] if key=="org"=>*address,
                    _=>return Err("Atari 800 Program requires exactly one unsigned org Option".to_string()),
                };
                check_memory(address,bytes.len(),None)?;
                self.machine.load_program(address as u16,&bytes)?;
                Ok(MachineResponse::Ack)
            }
            MachineCommand::AttachMedia{slot:MediaSlot::Disk{drive},kind:MediaKind::Disk,name,bytes,options}=>{
                if !options.is_empty() {
                    return Err("Atari disks accept no Options".to_string());
                }
                self.machine.attach_disk_in(drive, media_extension(name.as_deref())?, &bytes)?;
                Ok(MachineResponse::Ack)
            }
            MachineCommand::DetachMedia{slot:MediaSlot::Disk{drive}}=>{self.machine.eject_disk_in(drive)?;Ok(MachineResponse::Ack)}
            MachineCommand::ExportDisk{drive,only_if_modified}=>{
                if drive>1 {return Err("Atari SIO supports only D1:/D2:".to_string());}
                Ok(MachineResponse::DiskImage(if only_if_modified && !self.machine.disk_dirty_in(drive) {None} else {self.machine.export_disk_in(drive)}))
            }
            MachineCommand::MarkDiskSaved{drive}=>{self.machine.mark_disk_saved_in(drive)?;Ok(MachineResponse::Ack)}
            MachineCommand::AttachMedia{slot:MediaSlot::Cartridge{slot:0},kind:MediaKind::Cartridge,name,bytes,options}=>{
                if !options.is_empty() {return Err("Atari cartridges accept no Options; banked dumps require CAR headers".to_string());}
                self.machine.attach_cartridge(media_extension(name.as_deref())?,&bytes)?;
                Ok(MachineResponse::Ack)
            }
            MachineCommand::DetachMedia{slot:MediaSlot::Cartridge{slot:0}}=>{self.machine.eject_cartridge();Ok(MachineResponse::Ack)}
            MachineCommand::AttachMedia{slot:MediaSlot::Tape,kind:MediaKind::Tape,name,bytes,options}=>{
                if !media_extension(name.as_deref())?.eq_ignore_ascii_case("cas") ||
                    (!options.is_empty() && !matches!(options.as_slice(), [NamedValue {key, value:DebugValue::U64(1)}] if key=="tape_speed")) {
                    return Err("Atari cassette accepts standard CAS at tape_speed=1 only".to_string());
                }
                self.machine.attach_tape(&bytes)?;
                Ok(MachineResponse::Ack)
            }
            MachineCommand::DetachMedia{slot:MediaSlot::Tape}=>{self.machine.eject_tape();Ok(MachineResponse::Ack)}
            MachineCommand::KeyEvent{key,pressed}=>{self.key(&key,pressed)?;Ok(MachineResponse::Ack)}
            MachineCommand::SetBorder{enabled}=>{self.border=enabled;Ok(MachineResponse::Ack)}
            MachineCommand::RenderFrame=>{
                let (mut width,mut height,mut rgb)=self.machine.render_frame().expect("ANTIC frame");
                if !self.border {
                    let mut cropped=Vec::with_capacity(320*192*3);
                    for row in 24..216 {let start=(row*384+32)*3;cropped.extend_from_slice(&rgb[start..start+320*3]);}
                    width=320;height=192;rgb=cropped;
                }
                Ok(MachineResponse::Frame{width,height,rgb})
            }
            MachineCommand::DrainAudio=>Ok(MachineResponse::AudioPcm{channels:1,sample_rate:machine::AUDIO_SAMPLE_RATE,samples:self.machine.drain_audio()}),
            MachineCommand::SaveState=>{
                let bytes=bincode::serde::encode_to_vec(InputState{magic:*b"A8IN",machine:self.machine.save_state(),held:self.held.clone()},bincode::config::standard())
                    .map_err(|e|format!("cannot encode Atari 800 input state: {e}"))?;
                Ok(MachineResponse::StateBlob(bytes))
            }
            MachineCommand::LoadState{bytes}=>{
                if bytes.len()>67_108_864{return Err("Atari 800 state exceeds 64 MiB".to_string());}
                let (state,used):(InputState,usize)=bincode::serde::decode_from_slice(&bytes,bincode::config::standard().with_limit::<67_108_864>())
                    .map_err(|e|format!("invalid Atari 800 input state: {e}"))?;
                let names=key_names(self.machine.profile());
                if used!=bytes.len() || state.magic!=*b"A8IN" || state.held.iter().any(|name|!names.iter().any(|key|key.name==*name)){
                    return Err("invalid Atari 800 input state header, keys or trailing bytes".to_string());
                }
                self.machine.load_state(&state.machine)?;
                self.held=state.held;
                Ok(MachineResponse::Ack)
            }
            MachineCommand::GetMediaSnapshot=>Ok(MachineResponse::MediaSnapshot(self.media())),
            MachineCommand::Debug(command)=>Ok(MachineResponse::Debug(self.debug(command)?)),
            MachineCommand::TakePortHit=>Ok(MachineResponse::PortHit(None)),
            _=>Err("unsupported Atari operation: raw/XEX Program, ATR/XFD on D1:/D2:, CAS playback, standard/XEGS cartridges, keyboard, digital joysticks, State and the debug contract are implemented".to_string()),
        }
    }
}
fn media_extension(name: Option<&str>) -> Result<&str, String> {
    name.and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .filter(|extension| !extension.is_empty())
        .ok_or_else(|| "Atari Media requires a filename extension".to_string())
}
fn check_memory(address: u64, len: usize, space: Option<&str>) -> Result<(), String> {
    if address > 0xffff || len > 65536 || space.is_some_and(|s| s != "cpu") {
        return Err(
            "Atari 800 memory request requires 16-bit address, at most 65536 bytes and cpu space"
                .to_string(),
        );
    }
    Ok(())
}
struct Registry {
    next: u64,
    instances: BTreeMap<u64, Instance>,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            next: 1,
            instances: BTreeMap::new(),
        })
    })
}
fn with_instance<T>(
    id: u64,
    f: impl FnOnce(&mut Instance) -> Result<T, String>,
) -> Result<T, String> {
    let mut registry = registry()
        .lock()
        .map_err(|_| "Atari 800 registry poisoned")?;
    f(registry
        .instances
        .get_mut(&id)
        .ok_or_else(|| format!("unknown Machine instance {id}"))?)
}
fn encode<T: Serialize>(result: Result<T, String>) -> RcCallResult {
    match result {
        Ok(value) => match plugin_api::encode_message(&value) {
            Ok(bytes) => RcCallResult::ok_bytes(bytes),
            Err(error) => error.into_call_result(),
        },
        Err(error) => RcCallResult::error_with_message(
            if error.starts_with("unsupported") {
                StatusCode::Unsupported
            } else if error.starts_with("unknown Machine instance") {
                StatusCode::NotFound
            } else {
                StatusCode::InvalidRequest
            },
            &error,
        ),
    }
}
unsafe extern "C" fn list_profiles(request: RcSlice) -> RcCallResult {
    plugin_api::guard::guarded_call("list_profiles", || {
        // SAFETY: caller keeps request storage alive for this call.
        match plugin_api::decode_message::<ListProfilesRequest>(unsafe { request.as_bytes() }) {
            Ok(_) => encode(Ok(ListProfilesResponse {
                profiles: PROFILES.into_iter().map(profile).collect(),
            })),
            Err(e) => e.into_call_result(),
        }
    })
}
unsafe extern "C" fn create_instance(request: RcSlice) -> RcCallResult {
    plugin_api::guard::guarded_call("create_instance", || {
        // SAFETY: caller keeps request storage alive for this call.
        let request =
            match plugin_api::decode_message::<MachineCreateRequest>(unsafe { request.as_bytes() })
            {
                Ok(r) => r,
                Err(e) => return e.into_call_result(),
            };
        encode((|| {
            let instance = Instance::create(request)?;
            let metadata = instance.metadata();
            let mut registry = registry()
                .lock()
                .map_err(|_| "Atari 800 registry poisoned")?;
            let id = registry.next;
            registry.next = id
                .checked_add(1)
                .ok_or("Atari 800 instance handles exhausted")?;
            registry.instances.insert(id, instance);
            Ok(MachineCreateResponse {
                instance: id,
                metadata,
            })
        })())
    })
}
unsafe extern "C" fn destroy_instance(id: u64) -> RcStatus {
    plugin_api::guard::guarded(
        "destroy_instance",
        RcStatus::from_code(StatusCode::InternalError),
        || {
            if registry()
                .lock()
                .expect("Atari 800 registry poisoned")
                .instances
                .remove(&id)
                .is_some()
            {
                RcStatus::ok()
            } else {
                RcStatus::from_code(StatusCode::NotFound)
            }
        },
    )
}
unsafe extern "C" fn instance_call(id: u64, request: RcSlice) -> RcCallResult {
    plugin_api::guard::guarded_call("instance_call", || {
        // SAFETY: caller keeps request storage alive for this call.
        let request = match plugin_api::decode_message::<MachineInstanceCallRequest>(unsafe {
            request.as_bytes()
        }) {
            Ok(r) => r,
            Err(e) => return e.into_call_result(),
        };
        encode(with_instance(id, |instance| {
            Ok(MachineInstanceCallResponse {
                response: instance.command(request.command)?,
            })
        }))
    })
}
unsafe extern "C" fn free_buffer(buffer: RcBuffer) {
    plugin_api::guard::guarded("free_buffer", (), || {
        // SAFETY: this allocation originated in this Plugin.
        drop(unsafe { buffer.into_vec() });
    });
}
extern "C" fn step(id: u64) -> u32 {
    if plugin_api::guard::has_crashed() {
        return 4;
    }
    plugin_api::guard::guarded("step", 4, || {
        with_instance(id, |instance| Ok(instance.step())).expect("Atari 800 fast step failed")
    })
}
extern "C" fn halted(id: u64) -> u8 {
    if plugin_api::guard::has_crashed() {
        return 1;
    }
    plugin_api::guard::guarded("halted", 1, || {
        with_instance(id, |instance| Ok(u8::from(instance.machine.halted())))
            .expect("Atari 800 halted query failed")
    })
}
extern "C" fn pc_wide(id: u64) -> u64 {
    plugin_api::guard::guarded("pc_wide", 0, || {
        with_instance(id, |instance| Ok(instance.machine.pc())).expect("Atari 800 PC query failed")
    })
}
extern "C" fn pc(id: u64) -> u16 {
    pc_wide(id) as u16
}
const fn static_slice(bytes: &'static [u8]) -> RcSlice {
    RcSlice {
        ptr: bytes.as_ptr(),
        len: bytes.len(),
    }
}
struct StaticDescriptor(PluginDescriptorV1);
// SAFETY: descriptor only refers to immutable static data and functions.
unsafe impl Sync for StaticDescriptor {}
static VTABLE: MachinePluginVTableV1 = MachinePluginVTableV1 {
    size: MachinePluginVTableV1::expected_size(),
    list_profiles,
    create_instance,
    destroy_instance,
    instance_call,
    free_buffer,
    step,
    halted,
    pc,
    pc_wide,
};
static DESCRIPTOR: StaticDescriptor = StaticDescriptor(PluginDescriptorV1 {
    abi_major: ABI_MAJOR_V1,
    abi_minor: ABI_MINOR_V1,
    kind: PluginKind::Machine,
    id: static_slice(b"retroclawd.atari8.machine"),
    version: static_slice(build_info::version_tag!().as_bytes()),
    vtable: (&VTABLE as *const MachinePluginVTableV1).cast(),
});
/// # Safety
/// The returned descriptor is immutable static storage owned by this Plugin.
pub unsafe extern "C" fn entry_v1() -> *const PluginDescriptorV1 {
    &DESCRIPTOR.0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> MachineCreateRequest {
        let mut os = vec![0xea; 10240];
        os[10236..10238].copy_from_slice(&[0, 0xe0]);
        MachineCreateRequest {
            machine_id: Profile::Atari800.id().to_string(),
            region: None,
            options: Vec::new(),
            roms: vec![plugin_api::BinaryBlob {
                key: "rom_os".to_string(),
                bytes: os,
            }],
        }
    }
    #[test]
    fn names_are_canonical_unique_and_all_accepted() {
        let mut instance = Instance::create(request()).unwrap();
        let mut names = BTreeSet::new();
        for key in key_names(Profile::Atari800) {
            assert!(names.insert(plugin_api::normalize_key_name(&key.name)));
            instance.key(&key.name, true).unwrap();
            instance.key(&key.name, false).unwrap();
        }
        assert!(instance.key("Help", true).is_err(), "800 has no Help key");
    }
    #[test]
    fn each_profile_creates_with_its_own_rom_and_input_contract() {
        for model in PROFILES {
            let mut args = request();
            args.machine_id = model.id().to_string();
            args.roms[0].bytes = vec![0xea; model.os_size()];
            let mut instance = Instance::create(args).unwrap();
            assert_eq!(instance.machine.profile(), model);
            for key in key_names(model) {
                instance.key(&key.name, true).unwrap();
                instance.key(&key.name, false).unwrap();
            }
            assert_eq!(instance.key("Help", true).is_ok(), model.is_xl_xe());
            assert_eq!(instance.key("Joy3Fire1", true).is_ok(), !model.is_xl_xe());
            let mut wrong = request();
            wrong.machine_id = model.id().to_string();
            wrong.roms[0].bytes = vec![0; if model.is_xl_xe() { 10240 } else { 16384 }];
            assert!(Instance::create(wrong).is_err());
        }
    }

    #[test]
    fn tape_cartridge_and_second_disk_dispatch_restore_and_reject_atomically() {
        let mut instance = Instance::create(request()).unwrap();
        let attach = |slot, kind, name: &str, bytes| MachineCommand::AttachMedia {
            slot,
            kind,
            name: Some(name.into()),
            bytes,
            options: Vec::new(),
        };
        let xfd = vec![0xa5; 92160];
        instance
            .command(attach(
                MediaSlot::Disk { drive: 1 },
                MediaKind::Disk,
                "D2.XFD",
                xfd.clone(),
            ))
            .unwrap();
        instance
            .command(attach(
                MediaSlot::Cartridge { slot: 0 },
                MediaKind::Cartridge,
                "game.rom",
                vec![0x42; 8192],
            ))
            .unwrap();
        instance
            .command(attach(
                MediaSlot::Tape,
                MediaKind::Tape,
                "tape.cas",
                b"FUJI\0\0\0\0data\x01\0\0\0\x55".to_vec(),
            ))
            .unwrap();
        assert_eq!(instance.machine.read_memory(0xa000), 0x42);
        let saved = instance.machine.save_state();
        let MachineResponse::StateBlob(envelope) =
            instance.command(MachineCommand::SaveState).unwrap()
        else {
            panic!()
        };
        for (slot, kind, name) in [
            (MediaSlot::Disk { drive: 1 }, MediaKind::Disk, "bad.xfd"),
            (
                MediaSlot::Cartridge { slot: 0 },
                MediaKind::Cartridge,
                "bad.car",
            ),
            (MediaSlot::Tape, MediaKind::Tape, "bad.cas"),
        ] {
            assert!(instance.command(attach(slot, kind, name, vec![0])).is_err());
            assert_eq!(instance.machine.save_state(), saved);
        }
        for slot in [
            MediaSlot::Disk { drive: 1 },
            MediaSlot::Cartridge { slot: 0 },
            MediaSlot::Tape,
        ] {
            instance
                .command(MachineCommand::DetachMedia { slot })
                .unwrap();
        }
        instance
            .command(MachineCommand::LoadState { bytes: envelope })
            .unwrap();
        assert_eq!(instance.machine.read_memory(0xa000), 0x42);
        assert!(instance.machine.tape().is_some());
        assert!(matches!(
            instance.command(MachineCommand::ExportDisk { drive: 1, only_if_modified: false }).unwrap(),
            MachineResponse::DiskImage(Some(bytes)) if bytes == xfd
        ));
    }
    #[test]
    fn state_and_memory_failures_are_atomic_and_no_unimplemented_media_claimed() {
        let mut instance = Instance::create(request()).unwrap();
        instance.key("KeyA", true).unwrap();
        let MachineResponse::StateBlob(saved) =
            instance.command(MachineCommand::SaveState).unwrap()
        else {
            panic!()
        };
        instance.key("KeyA", false).unwrap();
        instance
            .command(MachineCommand::LoadState {
                bytes: saved.clone(),
            })
            .unwrap();
        assert!(instance.held.contains("KeyA"));
        let mut invalid = saved.clone();
        invalid.push(0);
        assert!(
            instance
                .command(MachineCommand::LoadState { bytes: invalid })
                .is_err()
        );
        let MachineResponse::StateBlob(after) =
            instance.command(MachineCommand::SaveState).unwrap()
        else {
            panic!()
        };
        assert_eq!(after, saved);
        assert!(
            instance
                .debug(DebugCommand::WriteMemory {
                    address: 0xbfff,
                    bytes: vec![1, 2],
                    space: None
                })
                .is_err()
        );
        assert_eq!(instance.machine.read_memory(0xbfff), 0);
        assert_eq!(
            profile(Profile::Atari800).media_kinds,
            vec![
                MediaKind::Program,
                MediaKind::Disk,
                MediaKind::Tape,
                MediaKind::Cartridge
            ]
        );
    }
    #[test]
    fn formatted_media_is_atomic_and_disk_survives_state_restore() {
        let mut instance = Instance::create(request()).unwrap();
        let program = |bytes| MachineCommand::AttachMedia {
            slot: MediaSlot::Other("program".to_string()),
            kind: MediaKind::Program,
            name: Some("fixture.XEX".to_string()),
            bytes,
            options: Vec::new(),
        };
        instance
            .command(program(vec![0xff, 0xff, 0, 0x20, 0, 0x20, 0xea]))
            .unwrap();
        assert_eq!(instance.machine.pc(), 0x2000);
        assert_eq!(instance.machine.read_memory(0x2000), 0xea);
        let before = instance.machine.save_state();
        assert!(
            instance
                .command(program(vec![0xff, 0xff, 0, 0xd8, 0, 0xd8, 0xea]))
                .is_err()
        );
        assert_eq!(instance.machine.save_state(), before);

        let disk = |bytes| MachineCommand::AttachMedia {
            slot: MediaSlot::Disk { drive: 0 },
            kind: MediaKind::Disk,
            name: Some("fixture.ATR".to_string()),
            bytes,
            options: Vec::new(),
        };
        let mut image = vec![0; 144];
        image[..6].copy_from_slice(&[0x96, 0x02, 8, 0, 128, 0]);
        instance.command(disk(image.clone())).unwrap();
        let before = instance.machine.save_state();
        assert!(instance.command(disk(vec![0; 16])).is_err());
        assert_eq!(instance.machine.save_state(), before);
        let MachineResponse::StateBlob(saved) =
            instance.command(MachineCommand::SaveState).unwrap()
        else {
            panic!()
        };
        instance
            .command(MachineCommand::DetachMedia {
                slot: MediaSlot::Disk { drive: 0 },
            })
            .unwrap();
        assert_eq!(instance.machine.export_disk(), None);
        instance
            .command(MachineCommand::LoadState { bytes: saved })
            .unwrap();
        let MachineResponse::DiskImage(exported) = instance
            .command(MachineCommand::ExportDisk {
                drive: 0,
                only_if_modified: false,
            })
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(exported, Some(image));
        assert!(matches!(
            instance
                .command(MachineCommand::ExportDisk {
                    drive: 0,
                    only_if_modified: true,
                })
                .unwrap(),
            MachineResponse::DiskImage(None)
        ));
    }

    #[test]
    fn rejects_wrong_region_mode_rom_and_cpu_without_mutation() {
        let mut invalid = request();
        invalid.region = Some("japan".to_string());
        assert!(Instance::create(invalid).is_err());
        let mut original = Instance::create(request()).unwrap();
        let MachineResponse::StateBlob(saved) =
            original.command(MachineCommand::SaveState).unwrap()
        else {
            panic!()
        };
        for change in ["region", "mode", "rom"] {
            let mut other = request();
            match change {
                "region" => other.region = Some("ntsc".to_string()),
                "mode" => other.options.push(NamedValue {
                    key: "execution_mode".to_string(),
                    value: DebugValue::Text("accurate".to_string()),
                }),
                "rom" => other.roms[0].bytes[0] ^= 1,
                _ => unreachable!(),
            }
            let mut other = Instance::create(other).unwrap();
            let before = other.machine.save_state();
            assert!(
                other
                    .command(MachineCommand::LoadState {
                        bytes: saved.clone()
                    })
                    .is_err()
            );
            assert_eq!(other.machine.save_state(), before);
        }
        let before = original.machine.save_state();
        assert!(
            original
                .debug(DebugCommand::OnCpu {
                    cpu: "secondary".to_string(),
                    command: Box::new(DebugCommand::GetRegisters)
                })
                .is_err()
        );
        assert_eq!(original.machine.save_state(), before);
    }
}
