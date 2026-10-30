// Vendored from liulifox233/Kirakira c451918023477620ff9055a2eb8a6714b947d9c1
// (crates/krkr-plugins/src/packinone.rs, tjsDataPack section and its
// container primitives), AGPL-3.0-or-later. CrySirror changes: install is
// `install`; writes go through the adapter's resumable atomic storage writes
// (`write_pack`); layer thumbnails are not captured (the engine's layers have
// no CPU bitmap), so makeDataPackThumb answers no thumbnail and logs it;
// KAGEX `kag.log` calls use the host log.
//! PackinOne `tjsDataPack`: Scripts.loadDataPack / saveDataPack /
//! makeDataPackThumb / makeDataPackDigest and Storages.saveOctet / loadOctet.
use std::sync::atomic::{AtomicBool, Ordering};

use crate::host::KrkrHost;
use crysirror_tjs2::{
    Result, TjsError, TjsErrorKind,
    runtime::{ObjectHandle, Runtime, TjsHost, Variant, tjs_ns0 as ns0},
};

fn ensure_global_object(runtime: &mut Runtime<KrkrHost>, name: &'static str) -> ObjectHandle {
    if let Some(h) = runtime.global_member(name).object_handle() {
        return h;
    }
    let h = runtime.alloc_ordinary_object();
    runtime.set_global_member(name, Variant::Object(h));
    h
}

/// CrySirror: storage writes are atomic jobs completed by the engine; the
/// calling script resumes once the write is published.
fn write_pack(runtime: &mut Runtime<KrkrHost>, name: &str, bytes: Vec<u8>) -> Result<()> {
    if !runtime.host().can_suspend {
        return Err(crate::natives::unsupported(
            &format!("data pack write `{name}` in this callback"),
            "resumable atomic write",
        ));
    }
    if runtime.host().writes.len() >= 64 {
        return Err(TjsError::runtime("data pack write queue budget"));
    }
    let job = runtime
        .host()
        .storage()?
        .begin_write(name, bytes.into())
        .map_err(|e| TjsError::runtime(format!("write `{name}`: {e:#}")))?;
    runtime.host_mut().writes.push_back(job);
    runtime.request_suspend();
    Ok(())
}

pub fn install(runtime: &mut Runtime<KrkrHost>) {
    install_storages_octet(runtime);
    install_data_pack(runtime);
}

fn install_storages_octet(runtime: &mut Runtime<KrkrHost>) {
    let storages = ensure_global_object(runtime, "Storages");
    runtime.register_object_native(storages, "saveOctet", storages_save_octet);
    runtime.register_object_native(storages, "loadOctet", storages_load_octet);
}

fn storages_save_octet(
    runtime: &mut Runtime<KrkrHost>,
    _this_obj: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    let name = args.first().cloned().unwrap_or_default().to_tjs_string()?;
    let bytes = match args.get(1) {
        Some(Variant::Octet(bytes)) => bytes.clone(),
        Some(value) => value.to_tjs_string()?.into_bytes(),
        None => Vec::new(),
    };
    write_pack(runtime, &name, bytes)?;
    Ok(Variant::Void)
}

fn storages_load_octet(
    runtime: &mut Runtime<KrkrHost>,
    _this_obj: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    let name = args.first().cloned().unwrap_or_default().to_tjs_string()?;
    match runtime.read_storage_binary(&name, "") {
        Ok(bytes) => Ok(Variant::Octet(bytes)),
        Err(_) => Ok(Variant::Void),
    }
}

// ---------------------------------------------------------------------------
// tjsDataPack: the DataPack container

/// The binary struct pack header (`crysirror-tjs2/src/runtime/builtins.rs`,
/// `BINARY_STRUCT_HEADER`) the engine's struct reader and the pack decoder
/// share. This port only *reads* it now: the reference container below is what
/// the writer emits.
const BINARY_STRUCT_HEADER: &[u8; 8] = b"KBAD100\0";

/// The footer this port's previous writer appended, laid out
/// `[u32 pack_offset][u32 pack_len][u32 seed][u8 version]["KDPK"]`
/// (M133). The reference container has no footer, so this is only parsed to
/// keep saves written by that build loading.
const DATA_PACK_FOOTER_MAGIC: &[u8; 4] = b"KDPK";
const DATA_PACK_FOOTER_VERSION: u8 = 1;
const DATA_PACK_FOOTER_LEN: usize = 17;

/// The raw LZ4 block size the reference frames at (`FUN_10051220`'s 0x1000
/// buffered stream is what feeds `LZ4CompressStream`).
const DATAPACK_LZ4_BLOCK: usize = 4096;

fn install_data_pack(runtime: &mut Runtime<KrkrHost>) {
    // tjsDataPack attaches these to the Scripts object (games call
    // `Scripts.loadDataPack(...)`); also expose them as globals for safety.
    let scripts = ensure_global_object(runtime, "Scripts");
    let global = runtime.global_handle();
    // The engine's canonical `loadDataPack` resolves a name without a `.pbd`
    // extension to `name.pbd` (the `PSDInfo.loadPBD` spelling). The reference
    // `tjsDataPack.dll` decodes the storage name it is handed, and KAGEX hands
    // it the bookmark's own file name — `BookMarkIO_DataPack.load` receives
    // `<saveDataLocation>data0.jpg` from `getBookMarkFileNameAtNum` — which the
    // `.pbd` rule would not find. The wrapper tries the exact name first and
    // falls back to the previous implementation (the engine's, also on a
    // `Plugins.link` re-registration) for every other spelling.
    let previous = runtime.object_member(scripts, "loadDataPack");
    for target in [scripts, global] {
        runtime.register_object_native(target, "saveDataPack", save_data_pack);
        runtime.register_object_native(target, "makeDataPackThumb", make_data_pack_thumb);
        runtime.register_object_native(target, "makeDataPackDigest", make_data_pack_digest);
        let previous = previous.clone();
        runtime.register_object_native(
            target,
            "loadDataPack",
            move |runtime: &mut Runtime<KrkrHost>,
                  this_obj: Option<ObjectHandle>,
                  args: Vec<Variant>| {
                load_data_pack(runtime, this_obj, args, &previous)
            },
        );
    }
}

/// `Scripts.loadDataPack(name[, options])`.
///
/// The reference `tjsDataPack.dll` reads the storage name it is handed; the
/// engine's implementation rewrites anything without a `.pbd` extension to
/// `name.pbd`. The exact name wins here, and the previous implementation —
/// the engine's, when this bundle is registered over it — handles the rest.
fn load_data_pack(
    runtime: &mut Runtime<KrkrHost>,
    _this_obj: Option<ObjectHandle>,
    args: Vec<Variant>,
    previous: &Variant,
) -> Result<Variant> {
    let name = args.first().cloned().unwrap_or_default().to_tjs_string()?;
    match runtime.read_storage_binary(&name, "") {
        Ok(bytes) => return decode_data_pack(runtime, &bytes, &name),
        // A lazily materialized Web asset must keep its request alive.
        Err(error) if error.kind == TjsErrorKind::ResourcePending => return Err(error),
        Err(_) => {}
    }
    if matches!(previous, Variant::Void) {
        // Registered without the engine's Scripts: resolve the `.pbd` alias
        // here, the engine's rule.
        let storage_name = data_pack_storage_name(&name);
        let bytes = runtime.read_storage_binary(&storage_name, "")?;
        return decode_data_pack(runtime, &bytes, &storage_name);
    }
    runtime.call_function(previous.clone(), args)
}

/// Decodes a `DataPack` storage: the reference container (optionally behind
/// the leading JPEG/PNG thumbnail a bookmark file starts with), a plain
/// `KBAD100` pack, or — for files this port wrote before the container switch
/// — the pack behind a `KDPK` footer.
fn decode_data_pack(
    runtime: &mut Runtime<KrkrHost>,
    bytes: &[u8],
    storage_name: &str,
) -> Result<Variant> {
    if let Some(pack) = reference_data_pack(bytes) {
        return decode_reference_data_pack(runtime, pack, storage_name);
    }
    if is_plain_data_pack(bytes) {
        return decode_plain_data_pack(runtime, bytes, storage_name);
    }
    if let Some(footer) = parse_data_pack_footer(bytes) {
        let start = footer.pack_offset as usize;
        let end = start.checked_add(footer.pack_len as usize);
        if let Some(payload) = end.and_then(|end| bytes.get(start..end))
            && is_plain_data_pack(payload)
        {
            return decode_plain_data_pack(runtime, payload, storage_name);
        }
    }
    Err(TjsError::runtime(format!(
        "Scripts.loadDataPack expected a binary data pack in `{storage_name}`"
    )))
}

/// The reference container behind its optional leading image: `TJS/` or
/// `TJS\` at offset 0, or right after the JPEG/PNG a bookmark file leads
/// with (`FUN_10050020` sniffs exactly these four magics).
fn reference_data_pack(bytes: &[u8]) -> Option<&[u8]> {
    if starts_with_ns0_magic(bytes) {
        return Some(bytes);
    }
    let end = image_end(bytes)?;
    let pack = bytes.get(end..)?;
    starts_with_ns0_magic(pack).then_some(pack)
}

fn starts_with_ns0_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(ns0::NS0_MAGIC_LE) || bytes.starts_with(ns0::NS0_MAGIC_BE)
}

/// Decodes the reference container: 16-byte header, IV, then the transform
/// chain — ChaCha over the whole stream, LZ4 raw-block framing — ahead of the
/// seeded value stream. The writer encrypts *after* compressing, so reading
/// undoes them in that order.
fn decode_reference_data_pack(
    runtime: &mut Runtime<KrkrHost>,
    pack: &[u8],
    storage_name: &str,
) -> Result<Variant> {
    let header = ns0::parse_ns0_header(pack).map_err(|error| {
        TjsError::runtime(format!(
            "Scripts.loadDataPack could not decode `{storage_name}`: {error}"
        ))
    })?;
    let iv_end = ns0::NS0_HEADER_SIZE + header.iv_length as usize;
    let iv = pack.get(ns0::NS0_HEADER_SIZE..iv_end).ok_or_else(|| {
        TjsError::runtime(format!(
            "Scripts.loadDataPack found a truncated data pack in `{storage_name}`"
        ))
    })?;
    let mut body = pack[iv_end..].to_vec();
    if header.cryptmode != 0 {
        datapack_chacha_apply(header.cryptmode, header.seed, iv, &mut body).map_err(|error| {
            TjsError::runtime(format!(
                "Scripts.loadDataPack could not decode `{storage_name}`: {error}"
            ))
        })?;
    }
    if header.compress == ns0::NS0_COMPRESS_LZ4 {
        body = datapack_lz4_deframe(&body).map_err(|error| {
            TjsError::runtime(format!(
                "Scripts.loadDataPack could not decode `{storage_name}`: {error}"
            ))
        })?;
    }
    runtime
        .decode_tjs_ns0_body(&body, header.seed, header.big_endian)
        .map_err(|error| {
            TjsError::runtime(format!(
                "Scripts.loadDataPack could not decode `{storage_name}`: {error}"
            ))
        })
}

fn decode_plain_data_pack(
    runtime: &mut Runtime<KrkrHost>,
    bytes: &[u8],
    storage_name: &str,
) -> Result<Variant> {
    if bytes.starts_with(BINARY_STRUCT_HEADER) {
        return runtime.decode_binary_struct(bytes)?.ok_or_else(|| {
            TjsError::runtime(format!(
                "Scripts.loadDataPack could not decode `{storage_name}`"
            ))
        });
    }
    runtime.decode_tjs_ns0(bytes)?.ok_or_else(|| {
        TjsError::runtime(format!(
            "Scripts.loadDataPack could not decode `{storage_name}`"
        ))
    })
}

fn is_plain_data_pack(bytes: &[u8]) -> bool {
    bytes.starts_with(BINARY_STRUCT_HEADER) || starts_with_ns0_magic(bytes)
}

/// `Scripts.saveDataPack(name, data[, digest[, thumb]])`.
///
/// KAGEX's `BookMarkIO_DataPack.save` calls it with four arguments
/// (system/MainWindow.tjs object 474): `data` is the bookmark dictionary
/// (`id`/`core`/`user`/`history`), `digest` is `calcThumbnailSize()`'s
/// dictionary after `makeDataPackDigest` filled its `seed` (and carries
/// `compress`/`cryptmode`/`iv`, set from `saveDataMode`), and `thumb` is
/// `makeDataPackThumb`'s encoded image (or void). The file is the reference
/// container — `[thumbnail image][16-byte header][iv][LZ4-framed and/or
/// ChaCha-encrypted body]`, no offset footer: the image's own chunk structure
/// ends the thumbnail and the body follows the header. The writer runs the
/// same chain the reference does (`FUN_10052090`): the body is serialized
/// with the seeded checker, framed in raw LZ4 blocks when `compress` is set,
/// then ChaCha-encrypted when `cryptmode` is set.
fn save_data_pack(
    runtime: &mut Runtime<KrkrHost>,
    _this_obj: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    let name = args.first().cloned().unwrap_or_default().to_tjs_string()?;
    let Some(data) = args.get(1).and_then(|value| value.object_handle()) else {
        // The reference raises `datapack type check failed: %1` for a subject
        // that is not a struct (PackinOne.dll tjsDataPack string pool).
        return Err(TjsError::runtime(format!(
            "saveDataPack: datapack type check failed: {name}"
        )));
    };
    let options = data_pack_options(runtime, args.get(2));
    let thumb = pack_thumbnail(runtime, args.get(3));
    let mut body = runtime.encode_tjs_ns0_body(&Variant::Object(data), options.seed, false)?;
    if options.compress == ns0::NS0_COMPRESS_LZ4 {
        body = datapack_lz4_frame(&body);
    }
    if options.cryptmode != 0 {
        datapack_chacha_apply(options.cryptmode, options.seed, &options.iv, &mut body)?;
    }
    let mut bytes = Vec::with_capacity(
        thumb.as_ref().map_or(0, Vec::len) + ns0::NS0_HEADER_SIZE + options.iv.len() + body.len(),
    );
    if let Some(thumb) = &thumb {
        bytes.extend_from_slice(thumb);
    }
    append_data_pack_header(&mut bytes, &options);
    bytes.extend_from_slice(&body);
    write_pack(runtime, &name, bytes)?;
    Ok(Variant::Void)
}

/// The header's digest-driven knobs (`BookMarkIO_DataPack`):
/// `compress` picks the `4s0` LZ4 framing, `cryptmode` the ChaCha parameter
/// set, `iv` the salt string (stored UTF-16LE plus its NUL terminator, the
/// byte form the real `m144-*.ksd` artifacts carry) and `seed` — when the
/// game leaves it zero — the writer's `"TJS"` default.
struct DataPackOptions {
    seed: u32,
    compress: u8,
    cryptmode: u16,
    iv: Vec<u8>,
}

fn data_pack_options(runtime: &Runtime<KrkrHost>, digest: Option<&Variant>) -> DataPackOptions {
    let handle = digest.and_then(|value| value.object_handle());
    let member = |name: &str| {
        handle
            .map(|handle| runtime.object_member(handle, name))
            .unwrap_or(Variant::Void)
    };
    let seed = digest_seed(runtime, digest)
        .filter(|seed| *seed != 0)
        .unwrap_or(ns0::NS0_DEFAULT_SEED);
    let compress = match member("compress") {
        Variant::Integer(value) if value != 0 => ns0::NS0_COMPRESS_LZ4,
        Variant::Real(value) if value != 0.0 => ns0::NS0_COMPRESS_LZ4,
        _ => ns0::NS0_COMPRESS_STORE,
    };
    let cryptmode = match member("cryptmode") {
        Variant::Integer(value) if (1..=6).contains(&value) => value as u16,
        _ => 0,
    };
    let iv = match member("iv") {
        // The reference stores the salt string as UTF-16LE *including its
        // NUL terminator* (`m144-*.ksd`: iv "kiri" → ivlen 10 = 2*(4+1); the
        // 20-character title artifact → 42 = 2*(20+1)).
        Variant::String(text) => {
            let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes.extend_from_slice(&[0, 0]);
            bytes
        }
        _ => Vec::new(),
    };
    DataPackOptions {
        seed,
        compress,
        cryptmode,
        iv,
    }
}

/// Writes the 16-byte header and the IV bytes (`FUN_1004e0d0`).
fn append_data_pack_header(out: &mut Vec<u8>, options: &DataPackOptions) {
    let mut header = [0u8; ns0::NS0_HEADER_SIZE];
    header[0..4].copy_from_slice(ns0::NS0_MAGIC_LE);
    header[4] = options.compress;
    header[5..8].copy_from_slice(b"s0\0");
    header[8..12].copy_from_slice(&options.seed.to_le_bytes());
    header[12..14].copy_from_slice(&options.cryptmode.to_le_bytes());
    header[14..16].copy_from_slice(&(options.iv.len() as u16).to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&options.iv);
}

/// `Scripts.makeDataPackThumb(layer, ext, quality, component)`.
///
/// `BookMarkIO_DataPack` picks the thumbnail format from `saveThumbnail` —
/// `jpg` for 3, `png` otherwise, `kdt` when thumbnails are off — and the save
/// path passes the captured layer to this function (object 474) before handing
/// its result to `saveDataPack`. `kdt` means "no thumbnail", so it produces
/// none; the other extensions get a baseline JPEG or a PNG of the layer's main
/// bitmap (the reference's own encoder is IJG, so the *bytes* differ, the
/// decodable image does not).
fn make_data_pack_thumb(
    runtime: &mut Runtime<KrkrHost>,
    _this_obj: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    let extension = args
        .get(1)
        .map(Variant::to_tjs_string)
        .transpose()?
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("kdt") {
        return Ok(Variant::Void);
    }
    let quality = args
        .get(2)
        .and_then(|value| value.to_integer().ok())
        .filter(|quality| (1..=100).contains(quality))
        .unwrap_or(75) as u8;
    let jpeg = !extension.eq_ignore_ascii_case("png");
    let Some(layer) = args.first().and_then(|value| value.object_handle()) else {
        return Ok(Variant::Void);
    };
    // CrySirror: no CPU bitmap of an engine layer; the save carries no
    // leading image (the reference's own fallback when capture fails).
    let _ = (layer, jpeg, quality);
    runtime.host_mut().log(
        "PackinOne.dll: makeDataPackThumb has no layer capture in CrySirror; saving without a thumbnail",
    );
    Ok(Variant::Void)
}

/// `Scripts.makeDataPackDigest(subject, seed, key[, flag])`.
///
/// KAGEX derives save key material from it: `BookMarkIO_DataPack.save` calls
/// `makeDataPackDigest(data, System.getTickCount() & 0xffffffff, saveDataID)`
/// and stores the result as the digest dictionary's `seed` member (object
/// 474); `Initialize.tjs`'s `MakeLockKey` and `gridchain.calchash` use the
/// four-argument form. The reference feeds the *big-endian* value
/// serialization of subject then key — including the per-value checker bytes,
/// zero for the zero seed the digest state starts with (`FUN_10053500`,
/// `FUN_10053450` over `VariantDigestState`/`XXH32Hasher`) — through
/// `XXH32(data, seed)` — the value a real-engine probe pinned
/// (`makeDataPackDigest(data, 7, "probe") = 882327524` for the `%["id" => ...]`
/// dictionary the M142 harness used). `flag = 1` selects the DLL's Blake2s
/// hasher (`FUN_10048910`, parameter block `[8, 4, 1, 1, 0, ...]`); this port
/// answers the XXH32 value either way and logs the substitution once.
fn make_data_pack_digest(
    runtime: &mut Runtime<KrkrHost>,
    _this_obj: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    static BLAKE2S_WARNED: AtomicBool = AtomicBool::new(false);
    let mut preimage = Vec::new();
    if let Some(subject) = args.first() {
        preimage.extend_from_slice(&digest_serialization(runtime, subject)?);
    }
    let seed = args
        .get(1)
        .and_then(|value| value.to_integer().ok())
        .unwrap_or(0) as u32;
    let key = args.get(2).cloned().unwrap_or_default();
    preimage.extend_from_slice(&digest_serialization(runtime, &key)?);
    let flag = args
        .get(3)
        .and_then(|value| value.to_integer().ok())
        .unwrap_or(0);
    if flag != 0 && !BLAKE2S_WARNED.swap(true, Ordering::Relaxed) {
        runtime.host_mut().log(
            "PackinOne.dll: makeDataPackDigest(flag=1) asks for Blake2s upstream; \
             this port answers XXH32 instead",
        );
    }
    Ok(Variant::Integer(i64::from(xxh32(&preimage, seed))))
}

/// One digest operand: the `TJS/ns0` big-endian serialization with the zero
/// seed's checker bytes and *without* the trailing final check (`FUN_10053450`
/// writes a single value into the digest state).
fn digest_serialization(runtime: &Runtime<KrkrHost>, value: &Variant) -> Result<Vec<u8>> {
    let mut body = runtime.encode_tjs_ns0_body(value, 0, true)?;
    body.truncate(body.len() - 4);
    Ok(body)
}

/// The digest dictionary's `seed`, the value `makeDataPackDigest` produced.
fn digest_seed(runtime: &Runtime<KrkrHost>, digest: Option<&Variant>) -> Option<u32> {
    let handle = digest.and_then(|value| value.object_handle())?;
    match runtime.object_member(handle, "seed") {
        Variant::Integer(seed) => Some(seed as u32),
        _ => None,
    }
}

/// The thumbnail image the save file should lead with.
///
/// The save path hands over `makeDataPackThumb`'s `Octet`; KAGEX's rewrite
/// path passes the *file name* it is rewriting (object 476 →
/// `saveDataPack(a0, a1, l0, a0)` in `rewriteBookMarkToFile`), which reuses
/// the pack's existing thumbnail; a raw layer is accepted the same way
/// `makeDataPackThumb` takes one (as a JPEG, the `saveThumbnail = 3` default).
fn pack_thumbnail(runtime: &mut Runtime<KrkrHost>, thumb: Option<&Variant>) -> Option<Vec<u8>> {
    match thumb {
        Some(Variant::Octet(bytes)) if !bytes.is_empty() => Some(bytes.clone()),
        Some(Variant::String(name)) => read_pack_thumbnail(runtime, name),
        // CrySirror: a raw layer cannot be captured (see makeDataPackThumb).
        Some(_) => None,
        None => None,
    }
}

/// The image a previous save leads with, so a rewrite that passes its own
/// file name keeps the slot's picture (`BookMarkIO_Standard.rewrite` loads the
/// old image and re-saves the layer; the DataPack path's rewrite simply hands
/// the file back to `saveDataPack`). The image ends where its own chunk
/// structure says — that is how the reference recovers the header that
/// follows it — with the old `KDPK` footer's offset as the fallback for files
/// this port wrote before the container switch.
fn read_pack_thumbnail(runtime: &mut Runtime<KrkrHost>, name: &str) -> Option<Vec<u8>> {
    if name.is_empty() {
        return None;
    }
    let bytes = match runtime.read_storage_binary(name, "") {
        Ok(bytes) => bytes,
        Err(_) => {
            let storage_name = data_pack_storage_name(name);
            runtime.read_storage_binary(&storage_name, "").ok()?
        }
    };
    let offset = image_end(&bytes).or_else(|| {
        let footer = parse_data_pack_footer(&bytes)?;
        Some(footer.pack_offset as usize)
    })?;
    (offset > 0 && offset <= bytes.len()).then(|| bytes[..offset].to_vec())
}

struct DataPackFooter {
    pack_offset: u32,
    pack_len: u32,
    #[allow(dead_code)]
    seed: u32,
}

fn parse_data_pack_footer(bytes: &[u8]) -> Option<DataPackFooter> {
    if bytes.len() < DATA_PACK_FOOTER_LEN {
        return None;
    }
    let footer = &bytes[bytes.len() - DATA_PACK_FOOTER_LEN..];
    if footer[12] != DATA_PACK_FOOTER_VERSION || &footer[13..17] != DATA_PACK_FOOTER_MAGIC {
        return None;
    }
    Some(DataPackFooter {
        pack_offset: u32::from_le_bytes(footer[0..4].try_into().ok()?),
        pack_len: u32::from_le_bytes(footer[4..8].try_into().ok()?),
        seed: u32::from_le_bytes(footer[8..12].try_into().ok()?),
    })
}

// ---------------------------------------------------------------------------
// Reference container primitives: XXH32, BLAKE2s, ChaCha and raw-block LZ4

const XXH32_PRIME1: u32 = 0x9E37_79B1;
const XXH32_PRIME2: u32 = 0x85EB_CA77;
const XXH32_PRIME3: u32 = 0xC2B2_AE3D;
const XXH32_PRIME4: u32 = 0x27D4_EB2F;
const XXH32_PRIME5: u32 = 0x1656_67B1;

fn xxh32_round(acc: u32, input: u32) -> u32 {
    acc.wrapping_add(input.wrapping_mul(XXH32_PRIME2))
        .rotate_left(13)
        .wrapping_mul(XXH32_PRIME1)
}

/// XXH32, the hash behind `makeDataPackDigest` and the ChaCha nonce
/// (`PackinOne.dll`'s `XXH32Hasher`; the primes sit in its `.text`).
fn xxh32(data: &[u8], seed: u32) -> u32 {
    let read = |offset: usize| {
        u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ])
    };
    let mut index = 0;
    let mut hash;
    if data.len() >= 16 {
        let mut v1 = seed.wrapping_add(XXH32_PRIME1).wrapping_add(XXH32_PRIME2);
        let mut v2 = seed.wrapping_add(XXH32_PRIME2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(XXH32_PRIME1);
        while index + 16 <= data.len() {
            v1 = xxh32_round(v1, read(index));
            v2 = xxh32_round(v2, read(index + 4));
            v3 = xxh32_round(v3, read(index + 8));
            v4 = xxh32_round(v4, read(index + 12));
            index += 16;
        }
        hash = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
    } else {
        hash = seed.wrapping_add(XXH32_PRIME5);
    }
    hash = hash.wrapping_add(data.len() as u32);
    while index + 4 <= data.len() {
        hash = hash
            .wrapping_add(read(index).wrapping_mul(XXH32_PRIME3))
            .rotate_left(17)
            .wrapping_mul(XXH32_PRIME4);
        index += 4;
    }
    while index < data.len() {
        hash = hash
            .wrapping_add(u32::from(data[index]).wrapping_mul(XXH32_PRIME5))
            .rotate_left(11)
            .wrapping_mul(XXH32_PRIME1);
        index += 1;
    }
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(XXH32_PRIME2);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(XXH32_PRIME3);
    hash ^= hash >> 16;
    hash
}

/// The BLAKE2s IV (SHA-256's constants) and message schedule.
const BLAKE2S_IV: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];
const BLAKE2S_SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];
/// The parameter block the reference initialises its cipher hash with:
/// 32-byte digest, then `4, 1, 1` and zeros (`FUN_10050bc0`; the digest
/// hasher's own block starts `8` at `FUN_10048910`).
const BLAKE2S_CIPHER_PARAM: [u8; 32] = [
    0x20, 4, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0,
];

fn blake2s_mixing(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, x: u32, y: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(x);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(y);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

fn blake2s_compress(state: &mut [u32; 8], block: &[u8], counter: u64, last: bool) {
    let mut message = [0u32; 16];
    for (index, word) in message.iter_mut().enumerate() {
        *word = u32::from_le_bytes([
            block[index * 4],
            block[index * 4 + 1],
            block[index * 4 + 2],
            block[index * 4 + 3],
        ]);
    }
    let mut working = [0u32; 16];
    working[..8].copy_from_slice(state);
    working[8..].copy_from_slice(&BLAKE2S_IV);
    working[12] ^= counter as u32;
    working[13] ^= (counter >> 32) as u32;
    if last {
        working[14] = !working[14];
    }
    for round in BLAKE2S_SIGMA {
        blake2s_mixing(
            &mut working,
            0,
            4,
            8,
            12,
            message[round[0]],
            message[round[1]],
        );
        blake2s_mixing(
            &mut working,
            1,
            5,
            9,
            13,
            message[round[2]],
            message[round[3]],
        );
        blake2s_mixing(
            &mut working,
            2,
            6,
            10,
            14,
            message[round[4]],
            message[round[5]],
        );
        blake2s_mixing(
            &mut working,
            3,
            7,
            11,
            15,
            message[round[6]],
            message[round[7]],
        );
        blake2s_mixing(
            &mut working,
            0,
            5,
            10,
            15,
            message[round[8]],
            message[round[9]],
        );
        blake2s_mixing(
            &mut working,
            1,
            6,
            11,
            12,
            message[round[10]],
            message[round[11]],
        );
        blake2s_mixing(
            &mut working,
            2,
            7,
            8,
            13,
            message[round[12]],
            message[round[13]],
        );
        blake2s_mixing(
            &mut working,
            3,
            4,
            9,
            14,
            message[round[14]],
            message[round[15]],
        );
    }
    for (index, value) in state.iter_mut().enumerate() {
        *value ^= working[index] ^ working[index + 8];
    }
}

/// BLAKE2s with an explicit parameter block (`FUN_10042c00` XORs the block
/// into the IV, which is the BLAKE2s initialisation).
fn blake2s(parts: &[&[u8]], param: &[u8; 32]) -> [u8; 32] {
    let mut state = BLAKE2S_IV;
    for (word, bytes) in state.iter_mut().zip(param.chunks_exact(4)) {
        *word ^= u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    let mut data = Vec::new();
    for part in parts {
        data.extend_from_slice(part);
    }
    let mut counter = 0u64;
    let mut offset = 0usize;
    // Every full block *except the last* is compressed as a non-final block;
    // an input whose length is a multiple of 64 (including zero) ends with
    // its final block already full — the reference's update/final pair never
    // appends a synthetic empty block.
    while offset + 64 < data.len() {
        let block = &data[offset..offset + 64];
        counter += 64;
        blake2s_compress(&mut state, block, counter, false);
        offset += 64;
    }
    let mut last = [0u8; 64];
    let tail = &data[offset..];
    last[..tail.len()].copy_from_slice(tail);
    counter += tail.len() as u64;
    blake2s_compress(&mut state, &last, counter, true);
    let mut digest = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    digest
}

/// The ChaCha filter's key hash (`FUN_10050bc0`): BLAKE2s-256 with the
/// cipher's parameter block over `[seed u32 LE][60 zero bytes][iv]`.
fn blake2s256(parts: &[&[u8]]) -> [u8; 32] {
    blake2s(parts, &BLAKE2S_CIPHER_PARAM)
}

/// The ChaCha parameter sets (`FUN_10051ec0`'s six cases): rounds (8/12/20,
/// i.e. ChaCha8/12/20) and the keystream batch in 64-byte blocks, whose
/// blocks 2..n are xorshift-stretched from the first instead of counter-stepped.
fn chacha_parameters(cryptmode: u16) -> Option<(usize, usize)> {
    match cryptmode {
        1 => Some((8, 16)),
        2 => Some((12, 8)),
        3 => Some((20, 4)),
        4 => Some((8, 1)),
        5 => Some((12, 1)),
        6 => Some((20, 1)),
        _ => None,
    }
}

fn chacha_quarter(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] ^= state[a];
    state[d] = state[d].rotate_left(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] ^= state[c];
    state[b] = state[b].rotate_left(12);
    state[a] = state[a].wrapping_add(state[b]);
    state[d] ^= state[a];
    state[d] = state[d].rotate_left(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] ^= state[c];
    state[b] = state[b].rotate_left(7);
}

fn chacha_block(input: &[u32; 16], rounds: usize) -> [u32; 16] {
    let mut working = *input;
    for _ in 0..rounds / 2 {
        chacha_quarter(&mut working, 0, 4, 8, 12);
        chacha_quarter(&mut working, 1, 5, 9, 13);
        chacha_quarter(&mut working, 2, 6, 10, 14);
        chacha_quarter(&mut working, 3, 7, 11, 15);
        chacha_quarter(&mut working, 0, 5, 10, 15);
        chacha_quarter(&mut working, 1, 6, 11, 12);
        chacha_quarter(&mut working, 2, 7, 8, 13);
        chacha_quarter(&mut working, 3, 4, 9, 14);
    }
    for index in 0..16 {
        working[index] = working[index].wrapping_add(input[index]);
    }
    working
}

/// The reference ChaCha stream (`BasicCryptFilter`): the state is
/// `sigma || BLAKE2s key || 64-bit counter || XXH32(iv, seed) || seed`, the
/// key is `BLAKE2s(seed LE || 60 zero bytes || iv)` with the cipher's
/// parameter block, and a batch of `blocks` blocks takes its first block from
/// the core (counter incremented once per batch) while the rest are per-word
/// xorshift (`v ^= v << 13; v ^= v >> 17; v ^= v << 5`, zero replaced by the
/// seed^nonce fallback) of the block before them (`FUN_10049240`,
/// `FUN_10050dc0`, `FUN_100510d0`).
struct ChaCha {
    key: [u32; 8],
    nonce: u32,
    seed: u32,
    rounds: usize,
    blocks: usize,
    fallback: u32,
    counter: u64,
    keystream: Vec<u8>,
    offset: usize,
}

impl ChaCha {
    fn new(cryptmode: u16, seed: u32, iv: &[u8]) -> Result<Self> {
        let (rounds, blocks) = chacha_parameters(cryptmode).ok_or_else(|| {
            TjsError::runtime(format!("unsupported ChaCha cryptmode {cryptmode}"))
        })?;
        let digest = blake2s256(&[&seed.to_le_bytes(), &[0u8; 60], iv]);
        let mut key = [0u32; 8];
        for (index, word) in key.iter_mut().enumerate() {
            *word = u32::from_le_bytes([
                digest[index * 4],
                digest[index * 4 + 1],
                digest[index * 4 + 2],
                digest[index * 4 + 3],
            ]);
        }
        let nonce = xxh32(iv, seed);
        let mixed = seed ^ nonce;
        let fallback = if mixed == 0 {
            if seed != 0 { seed } else { u32::MAX }
        } else {
            mixed
        };
        Ok(Self {
            key,
            nonce,
            seed,
            rounds,
            blocks,
            fallback,
            counter: 0,
            keystream: Vec::new(),
            offset: 0,
        })
    }

    fn refill(&mut self) {
        let counter = self.counter;
        self.counter += 1;
        let mut state = [0u32; 16];
        state[..4].copy_from_slice(&[0x6170_7865, 0x3320_646E, 0x7962_2D32, 0x6B20_6574]);
        state[4..12].copy_from_slice(&self.key);
        state[12] = counter as u32;
        state[13] = (counter >> 32) as u32;
        state[14] = self.nonce;
        state[15] = self.seed;
        let mut block = chacha_block(&state, self.rounds);
        self.keystream.clear();
        self.keystream.reserve(self.blocks * 64);
        for index in 0..self.blocks {
            if index > 0 {
                for word in block.iter_mut() {
                    let mut value = *word ^ (*word << 13);
                    value ^= value >> 17;
                    value ^= value << 5;
                    *word = if value == 0 { self.fallback } else { value };
                }
            }
            for word in block {
                self.keystream.extend_from_slice(&word.to_le_bytes());
            }
        }
        self.offset = 0;
    }

    fn apply(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            if self.offset == self.keystream.len() {
                self.refill();
            }
            *byte ^= self.keystream[self.offset];
            self.offset += 1;
        }
    }
}

/// Encrypts (or decrypts — the keystream is XOR) `data` in place with the
/// parameter set of `cryptmode`.
fn datapack_chacha_apply(cryptmode: u16, seed: u32, iv: &[u8], data: &mut [u8]) -> Result<()> {
    ChaCha::new(cryptmode, seed, iv)?.apply(data);
    Ok(())
}

/// `LZ4CompressStream` (`FUN_1004f800`): every 4096-byte chunk of the body
/// becomes `[u16 LE compressed length][raw LZ4 block]`. The framing is what
/// `4s0` names, and the reference decompresses each frame into one block
/// buffer, so frames must not exceed that size.
fn datapack_lz4_frame(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + body.len() / 255 + 16);
    for chunk in body.chunks(DATAPACK_LZ4_BLOCK) {
        let compressed = lz4_flex::block::compress(chunk);
        out.extend_from_slice(&(compressed.len() as u16).to_le_bytes());
        out.extend_from_slice(&compressed);
    }
    out
}

/// `LZ4DecompressStream` (`FUN_1004f700`): the inverse of the framing above.
fn datapack_lz4_deframe(framed: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < framed.len() {
        if index + 2 > framed.len() {
            return Err(TjsError::runtime("LZ4 frame length is truncated"));
        }
        let length = usize::from(u16::from_le_bytes([framed[index], framed[index + 1]]));
        index += 2;
        let limit = DATAPACK_LZ4_BLOCK + DATAPACK_LZ4_BLOCK / 255 + 16;
        if length == 0 || length > limit {
            return Err(TjsError::runtime("LZ4 frame has an implausible length"));
        }
        let block = framed
            .get(index..index + length)
            .ok_or_else(|| TjsError::runtime("LZ4 frame is truncated"))?;
        index += length;
        let start = out.len();
        out.resize(start + DATAPACK_LZ4_BLOCK, 0);
        let written =
            lz4_flex::block::decompress_into(block, &mut out[start..]).map_err(|error| {
                TjsError::runtime(format!("LZ4 frame does not decompress: {error}"))
            })?;
        out.truncate(start + written);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The leading image: chunk walkers (what the reference sniffer does) and the
// thumbnail encoders

/// Where the leading image ends and the container header starts. The
/// reference reads the file back through `JPGChunkStreamReader` /
/// `PNGChunkStreamReader`, which stop exactly at the image's own end marker
/// (`FUN_10050020` sniffs `TJS/`, `TJS\`, `\x89PNG` and `\xFF\xD8\xFF\xE0`).
fn image_end(bytes: &[u8]) -> Option<usize> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return jpeg_end(bytes);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png_end(bytes);
    }
    None
}

/// Walks JPEG markers to the EOI that ends the image. Entropy-coded data is
/// skipped with its stuffing (`FF 00`) and restart markers respected.
fn jpeg_end(bytes: &[u8]) -> Option<usize> {
    let mut index = 2;
    loop {
        if index + 2 > bytes.len() || bytes[index] != 0xFF {
            return None;
        }
        let mut marker = bytes[index + 1];
        while marker == 0xFF {
            index += 1;
            if index + 1 >= bytes.len() {
                return None;
            }
            marker = bytes[index + 1];
        }
        match marker {
            0x00 => return None,
            0xD8 => index += 2,
            0x01 | 0xD0..=0xD7 => index += 2,
            0xD9 => return Some(index + 2),
            0xDA => {
                index += 2;
                if index + 2 > bytes.len() {
                    return None;
                }
                let length = usize::from(u16::from_be_bytes([bytes[index], bytes[index + 1]]));
                index += length;
                loop {
                    if index + 1 >= bytes.len() {
                        return None;
                    }
                    if bytes[index] == 0xFF && bytes[index + 1] != 0x00 {
                        if (0xD0..=0xD7).contains(&bytes[index + 1]) {
                            index += 2;
                            continue;
                        }
                        break;
                    }
                    index += 1;
                }
            }
            _ => {
                if index + 4 > bytes.len() {
                    return None;
                }
                let length = usize::from(u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]));
                if length < 2 {
                    return None;
                }
                index += 2 + length;
            }
        }
    }
}

/// Walks the PNG chunk list to IEND.
fn png_end(bytes: &[u8]) -> Option<usize> {
    let mut index = 8;
    loop {
        let length_bytes = bytes.get(index..index + 4)?;
        let length = u32::from_be_bytes([
            length_bytes[0],
            length_bytes[1],
            length_bytes[2],
            length_bytes[3],
        ]) as usize;
        let kind = bytes.get(index + 4..index + 8)?;
        index = index.checked_add(12)?.checked_add(length)?;
        if index > bytes.len() {
            return None;
        }
        if kind == b"IEND" {
            return Some(index);
        }
    }
}

fn data_pack_storage_name(name: &str) -> String {
    if name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("pbd"))
    {
        name.to_string()
    } else {
        format!("{name}.pbd")
    }
}

