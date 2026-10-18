# Debuggers are Host modules, not Plugins

The browser, ZRCP and log Debuggers were once separate libraries behind their own C-ABI, although only the Host ever loaded them. They are now ordinary modules in `apps/retroclawd/src/debuggers/` behind cargo features (`debug-ui`, `debug-zrcp`, `debug-log`), reaching the Machine through an in-process `HostApi`. This removed a second ABI and its discovery mechanism; the cost is that a third party cannot add a Debugger without rebuilding the Host.
