# Shared Black Cyc formats

Concrete safe CPU Resource/Image plugins, derived from the four fully reviewed
MIT GARbro files pinned in SOURCE.json. They are usable without System-NNN or any
other VM. No original renderer, mixer, application or native executable runs.

GPK/GTB and VPK/VTB use the existing client-scoped companion resolver, checked
monotone entry ranges and read-at subrange readers. Signatureless archives need
an explicit format ID; opening one never copies the whole data archive. Over
2 GiB / 64-bit-footer layouts are explicitly unsupported.

DWQ unwraps PNG/JPEG through the shared decoder registry. Its BMP profiles retain
the original unusual plain RGB versus packed BGR channel order and top-down row
order. Packed pixels/masks use bounded zero-run plus previous-row XOR decoding;
mask coverage comes from palette luminance. Dimensions, row runs, palettes and
declared mask spans are checked; long loops observe the shared cancellation
context. Unsupported DIB/palette/profile variants fail.

VAW/WGQ return standard WAVE/Ogg readers for existing shared Audio. Wrapped
uncompressed data remains a subrange view. Delta decoding reserves its declared
output through the existing resource lease before allocation, rejects truncated
bits, and publishes immutable WAVE bytes. It does not play or mix audio.

These resource plugins do not establish a runnable System-NNN factory, correct
game presentation, all native script operations, original save support or audible
playback. Authored and installed-data validation are recorded separately.
