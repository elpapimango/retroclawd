# Plugin ABI

The plugin ABI policy (ABI major/minor, protocol version, buffer ownership, panic containment, release
layout) lives with the code it governs, in the shared-crates repository, checked out here as the submodule
`shared/`: **[`shared/docs/plugin-abi.md`](../shared/docs/plugin-abi.md)** (online:
<https://github.com/elpapimango/retroclawd-crates/blob/main/docs/plugin-abi.md>). Read it before changing
`shared/crates/plugin-api`.
