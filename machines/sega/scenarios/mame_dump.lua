local frames = {}
for f in string.gmatch(os.getenv("MAME_FRAMES") or "10", "%d+") do frames[tonumber(f)] = true end
local last = 0
for f, _ in pairs(frames) do if f > last then last = f end end
local n = 0
local out = os.getenv("MAME_OUT") or "."
emu.register_frame_done(function()
  n = n + 1
  if frames[n] then
    local mem = manager.machine.devices[":maincpu"].spaces["program"]
    local f = io.open(out .. "/m" .. n .. ".ram", "wb")
    for a = 0xFF0000, 0xFFFFFF do f:write(string.char(mem:read_u8(a))) end
    f:close()
    manager.machine.video:snapshot()
  end
  if n >= last then manager.machine:exit() end
end)
