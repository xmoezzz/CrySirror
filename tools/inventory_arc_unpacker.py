#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Inventory pinned decoder registrations plus explicitly reviewed Rust support.

The external source checkout is read-only. No fixture assets or key material are
copied; the output contains identifiers, source paths/hashes and explicit port state.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
IMAGE_MANIFEST = ROOT / "docs/arc-image-support.json"
PIN = "456834ecf2e5686813802c37efd829310485c57d"
RESOURCE_MANIFEST = ROOT / "docs/arc-resource-support.json"
PORTS = {}


# Reviewed metadata is the authority for support. Source declarations are only
# checked for agreement: adding a template, filename or FORMATS string cannot
# turn a pending decoder into an implementation claim.
def load_image_ports(path: Path = IMAGE_MANIFEST, *, expected_count=96):
    document = json.loads(path.read_text())
    if document.get("schema") != 1 or document.get("upstream_pin") != PIN:
        raise ValueError("image support manifest schema or pin differs")
    images = document.get("images")
    if not isinstance(images, list) or (expected_count is not None and len(images) != expected_count):
        raise ValueError("image support manifest count differs")
    ports = {}
    for image in images:
        decoder_id = image.get("id")
        if not isinstance(decoder_id, str) or not re.fullmatch(r"[a-z0-9-]+/[a-z0-9-]+", decoder_id):
            raise ValueError("invalid image support ID")
        if decoder_id in ports:
            raise ValueError("duplicate image support ID")
        if image.get("status") not in {"ported", "partial"}:
            raise ValueError("image support must explicitly be ported or partial")
        for key in ("implementation", "variants", "limits"):
            values = image.get(key)
            if not isinstance(values, list) or not values or not all(isinstance(value, str) and value for value in values):
                raise ValueError(f"image support {decoder_id} missing {key}")
        if not isinstance(image.get("evidence"), dict) or not image["evidence"].get("ordinary_tests"):
            raise ValueError(f"image support {decoder_id} missing test evidence")
        if image.get("registration_group") not in {"a", "b", "c", "d", "reused"}:
            raise ValueError("unknown image registration group")
        ports[decoder_id] = {**image, "kind": "image"}
    return ports


def strip_comments(text):
    token = re.compile(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'|//[^\n]*|/\*.*?\*/', re.S)
    return token.sub(lambda match: match[0] if match[0][0] in {'"', "'"} else " ", text)


def declared_formats(text):
    text = strip_comments(text)
    declaration = re.search(r"pub\s+const\s+FORMATS\s*:[^=]+?=\s*&\[(.*?)\]\s*;", text, re.S)
    if not declaration:
        raise ValueError("missing literal Rust FORMATS declaration")
    body = declaration[1]
    ids = re.findall(r'"([a-z0-9-]+/[a-z0-9-]+)"', body)
    remainder = re.sub(r'"[a-z0-9-]+/[a-z0-9-]+"|[\s,]', '', body)
    if remainder or not ids or len(ids) != len(set(ids)):
        raise ValueError("Rust FORMATS must be unique literal IDs")
    return set(ids)


def rust_function_body(text, name):
    declaration = re.search(r"pub(?:\s*\([^)]*\))?\s+fn\s+"+re.escape(name)+r"\b[^\{]*\{",text)
    if not declaration:
        raise ValueError(f"missing public Rust function {name}")
    start=declaration.end();depth=1
    token=re.compile(r'"(?:\\.|[^"\\])*"|[{}]')
    for match in token.finditer(text,start):
        if match[0]=='{':depth+=1
        elif match[0]=='}':depth-=1
        if depth==0:return text[start:match.start()]
    raise ValueError(f"unclosed Rust function {name}")


def validate_image_registration(ports, root: Path = ROOT):
    images = {key: value for key, value in ports.items() if value.get("kind") == "image"}
    if not images:
        return
    base = root / "crates/crysirror-formats-arc"
    text = strip_comments((base / "src/images.rs").read_text())
    registration = rust_function_body(text,"register_images")
    advertised = rust_function_body(text,"image_format_ids")
    actual = set()
    for group in "abcd":
        expected = {key for key, value in images.items() if value["registration_group"] == group}
        path = base / f"src/images_{group}.rs"
        if not path.exists() and not expected:
            continue
        declared = declared_formats(path.read_text())
        if declared != expected or actual & declared:
            raise ValueError(f"image manifest differs from native FORMATS group {group}")
        actual.update(declared)
        if not re.search(r"crate::images_"+group+r"::FORMATS",advertised):
            raise ValueError(f"native image IDs do not advertise group {group}")
        block = re.search(r"for\s+&id\s+in\s+crate::images_"+group+r"::FORMATS\s*\{(.*?)\}\)\)\?;", registration, re.S)
        if not block or not re.search(r"registry\.register_format", block[1]) or any(
            not re.search(r"\b"+field+r"\s*:\s*crate::images_"+group+r"::"+field, block[1])
            for field in ("probe", "decode")
        ):
            raise ValueError(f"native image group {group} is not registered with its decoder")
    reused = {key for key, value in images.items() if value["registration_group"] == "reused"}
    direct = set(re.findall(r'registry\.register_format\s*\(\s*Arc::new\s*\(\s*Plugin\s*\{\s*id\s*:\s*"([a-z0-9-]+/[a-z0-9-]+)"', registration))
    if direct != reused:
        raise ValueError("reused image manifest differs from native registrations")
    if any('"'+decoder_id+'"' not in advertised for decoder_id in reused):
        raise ValueError("reused image ID not advertised")
    actual.update(reused)
    if actual != set(images):
        raise ValueError("native registered image IDs differ from metadata")
    for image in images.values():
        for path in image["implementation"]:
            resolved = (base/path).resolve()
            if not resolved.is_relative_to(root.resolve()) or not resolved.is_file():
                raise ValueError("image implementation absent or outside repository")



def load_resource_ports(path: Path = RESOURCE_MANIFEST, *, expected_count=193):
    document = json.loads(path.read_text())
    if document.get("schema") != 1 or document.get("upstream_pin") != PIN:
        raise ValueError("resource support manifest schema or pin differs")
    resources = document.get("resources")
    if not isinstance(resources, list) or (expected_count is not None and len(resources) != expected_count):
        raise ValueError("resource support manifest count differs")
    ports = {}
    for resource in resources:
        decoder_id = resource.get("id")
        if not isinstance(decoder_id, str) or not re.fullmatch(r"[a-z0-9-]+/[a-z0-9-]+", decoder_id):
            raise ValueError("invalid resource support ID")
        if decoder_id in ports:
            raise ValueError("duplicate resource support ID")
        if resource.get("kind") not in {"archive", "audio", "file"}:
            raise ValueError("resource support kind must be archive, audio or file")
        if resource.get("status") not in {"ported", "partial"}:
            raise ValueError("resource support must explicitly be ported or partial")
        for key in ("implementation", "variants", "limits"):
            values = resource.get(key)
            if not isinstance(values, list) or not values or not all(isinstance(v, str) and v for v in values):
                raise ValueError(f"resource support {decoder_id} missing {key}")
        evidence = resource.get("evidence")
        if not isinstance(evidence, dict) or (not evidence.get("ordinary_tests") and not evidence.get("validation_boundary")):
            raise ValueError(f"resource support {decoder_id} missing ordinary test evidence")
        if not evidence.get("ordinary_tests") and resource["status"] != "partial":
            raise ValueError("unvalidated resource must retain explicit partial state")
        for key in ("ordinary_tests", "external_corpus_tests"):
            values = evidence.get(key, [])
            if not isinstance(values, list) or not all(isinstance(v, str) and v for v in values):
                raise ValueError("invalid resource test evidence references")
        if resource.get("registration_group") not in set("abcdefghi") | {"reused", "audio", "file"}:
            raise ValueError("unknown resource registration group")
        if not isinstance(resource.get("entrypoint"), str) or not resource["entrypoint"]:
            raise ValueError("resource entrypoint missing")
        ports[decoder_id] = resource
    return ports


def balanced(text, start, opening="{", closing="}"):
    if text[start] != opening:
        raise ValueError("native declaration delimiter")
    depth = 1
    token = re.compile(r'"(?:\\.|[^"\\])*"|[' + re.escape(opening + closing) + ']')
    for match in token.finditer(text, start + 1):
        if match[0] == opening:
            depth += 1
        elif match[0] == closing:
            depth -= 1
        if depth == 0:
            return text[start + 1:match.start()], match.end()
    raise ValueError("unclosed native declaration")


def legacy_declarations(text):
    text = strip_comments(text)
    declaration = re.search(r"pub\s*\([^)]*\)\s+static\s+FORMATS\s*:[^=]+?=\s*&\[", text)
    if not declaration:
        raise ValueError("missing native Legacy FORMATS declaration")
    body, _ = balanced(text, declaration.end() - 1, "[", "]")
    declarations = {}
    position = 0
    while position < len(body):
        record = re.match(r"[\s,]*Legacy\s*\{", body[position:])
        if not record:
            if body[position:].strip(" \r\n\t,"):
                raise ValueError("nonliteral Legacy FORMATS record")
            break
        fields, position = balanced(body, position + record.end() - 1)
        decoder_id = re.search(r'\bid\s*:\s*"([a-z0-9-]+/[a-z0-9-]+)"\s*,', fields)
        entrypoint = re.search(r"\bopen\s*:\s*(.*?)\s*,?\s*$", fields, re.S)
        if not decoder_id or not entrypoint:
            raise ValueError("native Legacy record needs id and parser open")
        decoder_id = decoder_id[1]
        if decoder_id in declarations:
            raise ValueError("duplicate native Legacy ID")
        expression = re.sub(r"\s+", "", entrypoint[1].rstrip(","))
        if not expression or re.search(r"\b(?:todo|unimplemented)!", expression):
            raise ValueError("native Legacy parser missing")
        declarations[decoder_id] = expression
    return declarations


def literal_ids(text, name="IDS"):
    text = strip_comments(text)
    declaration = re.search(r"\bconst\s+" + re.escape(name) + r"\s*:[^=]+?=\s*&\[", text)
    if not declaration:
        raise ValueError("missing native resource IDS declaration")
    body, _ = balanced(text, declaration.end() - 1, "[", "]")
    ids = re.findall(r'"([a-z0-9-]+/[a-z0-9-]+)"', body)
    remainder = re.sub(r'"[a-z0-9-]+/[a-z0-9-]+"|[\s,]', '', body)
    if remainder or not ids or len(ids) != len(set(ids)):
        raise ValueError("native resource IDS must be unique literals")
    return set(ids)


def validate_resource_registration(ports, root: Path = ROOT):
    resources = {key: value for key, value in ports.items() if value.get("kind") in {"archive", "audio", "file"}}
    if not resources:
        return
    base = root / "crates/crysirror-formats-arc"
    for resource in resources.values():
        for relative in resource["implementation"]:
            path = (base / relative).resolve()
            if not path.is_relative_to(root.resolve()) or not path.is_file():
                raise ValueError("resource implementation absent or outside repository")
    native = strip_comments((base / "src/lib.rs").read_text())
    register = re.sub(r"\s+", "", rust_function_body(native, "register"))
    archives = {key: value for key, value in resources.items() if value["kind"] == "archive"}
    legacy = {key: value for key, value in archives.items() if value["registration_group"] != "reused"}
    if legacy:
        aggregate = strip_comments((base / "src/legacy.rs").read_text())
        body = re.sub(r"\s+", "", rust_function_body(aggregate, "formats"))
        groups = set(value["registration_group"] for value in legacy.values())
        linked = re.findall(r"\b([a-i])::FORMATS", body)
        if set(linked) != groups or len(linked) != len(set(linked)):
            raise ValueError("native archive aggregate groups differ from resource manifest")
        for group in groups:
            declared = legacy_declarations((base / f"src/legacy/{group}.rs").read_text())
            expected = {key: value for key, value in legacy.items() if value["registration_group"] == group}
            if set(declared) != set(expected):
                raise ValueError(f"resource manifest differs from native archive FORMATS group {group}")
            if not re.search(r"\bmod\s+" + group + r"\s*;", aggregate):
                raise ValueError("native archive module is not linked")
            for decoder_id, resource in expected.items():
                if declared[decoder_id] != re.sub(r"\s+", "", resource["entrypoint"]):
                    raise ValueError(f"native archive parser differs from reviewed entrypoint: {decoder_id}")
                expression = declared[decoder_id]
                referenced = re.findall(r"(?:[a-z_][a-z0-9_]*::)*([a-z_][a-z0-9_]*)\(", expression)
                if not referenced:
                    referenced = [expression.rsplit("::", 1)[-1]]
                implementation = "\n".join(strip_comments((base / p).read_text()) for p in resource["implementation"] if (base / p).is_file())
                if not referenced or not any(re.search(r"\bfn\s+" + re.escape(symbol) + r"\s*\(", implementation) for symbol in referenced):
                    raise ValueError(f"reviewed native archive parser definition absent: {decoder_id}")
        if "forformatinlegacy::formats()" not in register or "LegacyFormat(format)" not in register or "registry.register(" not in register:
            raise ValueError("native archive aggregate is not registered")
        if "self.0.open(source,context)" not in re.sub(r"\s+", "", native):
            raise ValueError("native archive wrapper does not call its parser")
    direct = {key: value for key, value in archives.items() if value["registration_group"] == "reused"}
    for decoder_id, resource in direct.items():
        constructor = re.sub(r"\s+", "", resource["entrypoint"])
        if "registry.register(std::sync::Arc::new(" + constructor + "))?" not in register:
            raise ValueError(f"reviewed direct archive is not registered: {decoder_id}")
    for kind in ("audio", "file"):
        expected = {key: value for key, value in resources.items() if value["kind"] == kind}
        if not expected:
            continue
        filename = "audio" if kind == "audio" else "files"
        source = strip_comments((base / f"src/{filename}.rs").read_text())
        if literal_ids(source) != set(expected):
            raise ValueError(f"resource manifest differs from native {kind} IDS")
        body = re.sub(r"\s+", "", rust_function_body(source, "register_" + filename))
        advertised = re.sub(r"\s+", "", rust_function_body(source, kind + "_format_ids"))
        if "for&idinIDS" not in body or "registry.register(Arc::new(Format(id)))?" not in body or advertised != "IDS.to_vec()":
            raise ValueError(f"native {kind} IDs are not registered with their decoder")
        if "decode(self.0,path,&d,cx)" not in re.sub(r"\s+", "", source):
            raise ValueError(f"native {kind} registry does not call its decoder")
        if "register_" + filename not in native or "mod" + filename + ";" not in re.sub(r"\s+", "", native):
            raise ValueError(f"native {kind} registry is not exported")
        dispatch = re.search(r"\bfn\s+decode\s*\(\s*id\s*:", source)
        if not dispatch:
            raise ValueError(f"native {kind} decoder dispatch absent")
        match = re.search(r"\bmatch\s+id\s*\{", source[dispatch.start():])
        if not match:
            raise ValueError(f"native {kind} decoder match absent")
        handlers, _ = balanced(source, dispatch.start() + match.end() - 1)
        implemented = set(re.findall(r'"([a-z0-9-]+/[a-z0-9-]+)"\s*(?:\||=>)', handlers))
        if set(expected) - implemented:
            raise ValueError(f"native {kind} ID has no decoder branch: {sorted(set(expected) - implemented)}")
        if any(resource.get("entrypoint") != filename + "::Format::decode" for resource in expected.values()):
            raise ValueError(f"native {kind} dispatch differs from reviewed entrypoint")
    for resource in resources.values():
        for relative in resource["implementation"]:
            path = (base / relative).resolve()
            if not path.is_relative_to(root.resolve()) or not path.is_file():
                raise ValueError("resource implementation absent or outside repository")
        for field in ("ordinary_tests", "external_corpus_tests"):
            for reference in resource.get("evidence", {}).get(field, []):
                relative, _, function = reference.partition("::")
                path = (base / relative).resolve()
                if not path.is_relative_to(root.resolve()) or not path.is_file():
                    raise ValueError("resource evidence absent or outside repository")
                if function and not re.search(r"\bfn\s+" + re.escape(function) + r"\s*\(", strip_comments(path.read_text())):
                    raise ValueError("resource evidence function absent")


if IMAGE_MANIFEST.exists():
    PORTS.update(load_image_ports())
if RESOURCE_MANIFEST.exists():
    PORTS.update(load_resource_ports())


def inventory(source: Path, *, rust_root: Path = ROOT):
    source = source.expanduser().resolve()
    head = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if head != PIN:
        raise ValueError("arc_unpacker source HEAD differs from audited pin")
    dirty = subprocess.check_output(["git", "-C", str(source), "status", "--porcelain", "--untracked-files=no"], text=True).strip()
    if dirty:
        raise ValueError("arc_unpacker tracked source has local changes")
    validate_image_registration(PORTS, rust_root)
    validate_resource_registration(PORTS, rust_root)
    # Work only from tracked objects at HEAD, not generated/untracked decoder
    # files. Compare Git blob bytes as well as status (assume-unchanged can hide
    # worktree edits from git status).
    tree = subprocess.check_output(["git", "-C", str(source), "ls-tree", "-r", "-z", "HEAD", "--", "src"])
    contents, hashes = {}, {}
    for record in tree.split(b"\0"):
        if not record:
            continue
        descriptor, name = record.split(b"\t", 1)
        relative = name.decode("utf-8")
        if Path(relative).suffix not in {".cc", ".h"}:
            continue
        resolved = (source / relative).resolve()
        if not resolved.is_relative_to(source):
            raise ValueError("source file escapes reference root")
        data = resolved.read_bytes()
        blob = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        if blob != descriptor.split()[2].decode("ascii"):
            raise ValueError(f"source bytes differ from pinned HEAD: {relative}")
        contents[relative] = data.decode("utf-8")
        hashes[relative] = hashlib.sha256(data).hexdigest()
    entries = []
    def header_kind(path, seen=None):
        seen = set() if seen is None else seen
        path = path.resolve()
        if path in seen or not path.is_file():
            return set()
        if not path.is_relative_to(source):
            raise ValueError("decoder header escapes reference root")
        seen.add(path)
        relative = path.relative_to(source).as_posix()
        if relative not in contents:
            raise ValueError(f"decoder header absent from pinned source: {relative}")
        text = strip_comments(contents[relative])
        direct = set(re.findall(r'public\s+(?:\w+::)*Base(Archive|Image|Audio|File)Decoder', text))
        if direct:
            return direct
        inherited = set()
        for relative in re.findall(r'#include\s+"(dec/[^"\n]+\.h)"', text):
            inherited.update(header_kind(source / "src" / relative, seen))
        return inherited

    registration = re.compile(r'register_decoder\s*<\s*([\w:]+)\s*>\s*\(\s*"([^"\n]+)"', re.S)
    for relative, text in sorted(contents.items()):
        path = source / relative
        if path.suffix != ".cc" or "/dec/" not in str(path):
            continue
        header = path.with_suffix(".h")
        bases = header_kind(header)
        kind = next(iter(bases)).lower() if len(bases) == 1 else "other"
        for cls, decoder_id in registration.findall(strip_comments(text)):
            rust = PORTS.get(decoder_id, {"status": "pending", "implementation": [], "variants": [], "limits": ["Not ported; upstream registration is inventory only"]})
            if rust.get("kind", kind) != kind:
                raise ValueError(f"reviewed decoder kind differs from pinned source: {decoder_id}")
            entries.append({"id": decoder_id, "kind": kind, "upstream_class": cls, "source": relative,
                            "source_sha256": hashes[relative], "rust": rust})
    entries.sort(key=lambda entry: entry["id"])
    if len({entry["id"] for entry in entries}) != len(entries):
        raise ValueError("duplicate upstream decoder registration id")
    if set(PORTS) - {entry["id"] for entry in entries}:
        raise ValueError("ported decoder absent from pinned source")
    counts = {}
    kinds = {}
    for entry in entries:
        status = entry["rust"]["status"]
        counts[status] = counts.get(status, 0) + 1
        kinds[entry["kind"]] = kinds.get(entry["kind"], 0) + 1
    tree_hash = hashlib.sha256(json.dumps(hashes, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return {"schema": 1, "source": {"url": "https://github.com/vn-tools/arc_unpacker.git", "commit": head,
            "license": "GPL-3.0-or-later", "license_source": "LICENSE.md and per-file GPLv3-or-later headers",
            "complete_cpp_source_files": len(hashes), "cpp_source_manifest_sha256": tree_hash,
            "source_is_vendored": False}, "registered_upstream_decoders": len(entries),
            "rust_status_counts": counts, "upstream_kind_counts": kinds,
            "rust_registered_ids": sorted(decoder_id for decoder_id, port in PORTS.items() if port["status"] in {"ported", "partial"}),
            "scope": "Reviewed manifests describe concrete registered Rust resource/image decoders. Ported and partial are scoped implementation states, not proof of every title/layout, original VM compatibility, or complete-game behavior.",
            "image_support_manifest": "docs/arc-image-support.json",
            "resource_support_manifest": "docs/arc-resource-support.json",
            "rust_registered_counts": {kind: sum(entry["kind"] == kind and entry["rust"]["status"] in {"ported", "partial"} for entry in entries) for kind in sorted(kinds)},
            "source_files_sha256": hashes, "decoders": entries}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = inventory(args.source_root)
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps({"registered_upstream_decoders": result["registered_upstream_decoders"],
                      "rust_status_counts": result["rust_status_counts"], "upstream_kind_counts": result["upstream_kind_counts"]}))
