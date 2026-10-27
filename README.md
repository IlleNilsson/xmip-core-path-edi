# xmip-core-path-edi

The `edi` path technology, a technology of
[xmip-core-path](https://github.com/IlleNilsson/xmip-core-path). It carries
`EdiLanguage`, the `PathLanguage` for the language `edi`: a segment, element
and component of an EDIFACT or X12 interchange, read for promote, route and
process and written for demote. A path is
`TAG[+QUALIFIER][#N]/ELEMENT[/COMPONENT]`, every number from 1 — `BGM/2`,
`NAD+BY/2/1`, `REF#2/2`. The interchange is cut into segments by the
contract technologies' own syntax, once per Message however many paths read
it; a write replaces one component and writes the interchange again in its
own service characters, releasing what EDIFACT must release and refusing in
X12 a value that carries a separator, a new Stream (ADR-0013).
## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
