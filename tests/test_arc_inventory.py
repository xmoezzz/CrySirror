# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic source inventory tests. No source clone, game fixtures or Cargo."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("arc_inventory", ROOT / "tools/inventory_arc_unpacker.py")
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


class ArcInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="arc-inventory-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.git("init", "--quiet")
        self.write("src/dec/registry.h", 'template<typename T> bool register_decoder(const char *id);\n')
        self.write("src/dec/family/example.h", 'class Example : public BaseArchiveDecoder {};\n')
        self.relative = "src/dec/family/example.cc"
        self.original = '''auto a=register_decoder<Example>("fixture/ported");
auto b=register_decoder<Example>("fixture/partial");
auto c=register_decoder<Example>("fixture/pending");
'''
        self.write(self.relative, self.original)
        self.commit()
        self.pin = self.git("rev-parse", "HEAD").strip()
        self.ports = {name: {"status": status, "implementation": ["src/format.rs"] if status != "pending" else [],
                             "variants": [], "limits": []}
                      for name, status in [("fixture/ported", "ported"), ("fixture/partial", "partial"), ("fixture/pending", "pending")]}

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], text=True)

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self):
        self.git("add", ".")
        self.git("-c", "user.name=Inventory Test", "-c", "user.email=inventory@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "synthetic source")

    def inventory(self):
        with patch.object(module, "PIN", self.pin), patch.object(module, "PORTS", self.ports):
            return module.inventory(self.root)

    def test_helper_template_is_not_a_registration_and_pending_is_not_registered(self):
        result = self.inventory()
        self.assertEqual(result["registered_upstream_decoders"], 3)
        self.assertEqual(result["rust_status_counts"], {"ported": 1, "partial": 1, "pending": 1})
        self.assertEqual(result["rust_registered_ids"], ["fixture/partial", "fixture/ported"])
        self.assertEqual(result["upstream_kind_counts"], {"archive": 3})
        pending = next(item for item in result["decoders"] if item["id"] == "fixture/pending")
        self.assertEqual(pending["rust"]["implementation"], [])

    def test_wrong_pin_and_dirty_or_staged_changes_fail(self):
        with self.assertRaisesRegex(ValueError, "HEAD differs"):
            module.inventory(self.root)
        self.write(self.relative, self.original + "// dirty\n")
        with self.assertRaisesRegex(ValueError, "local changes"):
            self.inventory()
        self.git("add", self.relative)
        with self.assertRaisesRegex(ValueError, "local changes"):
            self.inventory()

    def test_duplicate_id_at_verified_head_fails(self):
        self.write(self.relative, self.original + 'auto duplicate=register_decoder<Example>("fixture/ported");\n')
        self.commit()
        self.pin = self.git("rev-parse", "HEAD").strip()
        with self.assertRaisesRegex(ValueError, "duplicate upstream decoder"):
            self.inventory()

    def test_assume_unchanged_does_not_hide_modified_source_bytes(self):
        self.git("update-index", "--assume-unchanged", self.relative)
        self.write(self.relative, self.original + "// hidden change\n")
        with self.assertRaisesRegex(ValueError, "source bytes differ"):
            self.inventory()

    def test_untracked_decoder_does_not_invent_pinned_coverage(self):
        self.write("src/dec/family/extra.cc", 'auto fake=register_decoder<Example>("fixture/untracked");\n')
        self.assertEqual(self.inventory()["registered_upstream_decoders"], 3)

    def test_commented_registration_and_helper_template_do_not_invent_ids(self):
        self.write(self.relative, self.original + """
// auto fake=register_decoder<Example>("fixture/comment");
/* auto fake2=register_decoder<Example>("fixture/block-comment"); */
template<typename T> bool register_decoder(const char *id);
""")
        self.commit()
        self.pin = self.git("rev-parse", "HEAD").strip()
        result = self.inventory()
        self.assertEqual(result["registered_upstream_decoders"], 3)
        self.assertNotIn("fixture/comment", {entry["id"] for entry in result["decoders"]})


class ImageSupportManifestTests(unittest.TestCase):
    def test_checked_in_manifest_matches_actual_native_registration_contract(self):
        ports = module.load_image_ports()
        self.assertEqual(len(ports), 96)
        module.validate_image_registration(ports)
        saved = __import__('json').loads((ROOT / 'crates/crysirror-formats-arc/upstream-formats.json').read_text())
        self.assertEqual(saved['registered_upstream_decoders'], 289)
        self.assertEqual(saved['rust_registered_counts']['image'], 96)
        self.assertEqual(saved['rust_registered_counts']['archive'], 5)
        self.assertEqual(saved['rust_status_counts']['pending'], 188)
        self.assertEqual(saved['source']['complete_cpp_source_files'], 870)
        self.assertEqual(len(saved['source_files_sha256']), 870)
        self.assertEqual({d['id'] for d in saved['decoders'] if d['kind'] == 'image'}, set(ports))
        self.assertEqual({d['id'] for d in saved['decoders'] if d['rust']['status'] in {'ported', 'partial'}}, set(module.PORTS))

    def test_partial_boundaries_and_archive_state_are_preserved(self):
        import json
        ports = module.load_image_ports()
        for name, text in [('bgi/cbg', 'CBGv2'), ('google/webp', 'max7'), ('jpeg/jpeg', 'max1'),
                           ('silky/akb', 'stem'), ('playstation/gtf', 'multi-texture'), ('leaf/bjr', 'ASCII')]:
            self.assertEqual(ports[name]['status'], 'partial')
            self.assertIn(text, ' '.join(ports[name]['limits']))
        self.assertEqual(module.PORTS['nscripter/nsa']['status'], 'partial')
        self.assertIn('SPB conversion (compression 1) explicitly unsupported', module.PORTS['nscripter/nsa']['limits'])
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'support.json'
            document = json.loads(module.IMAGE_MANIFEST.read_text())
            next(i for i in document['images'] if i['id'] == 'bgi/cbg')['limits'] = []
            path.write_text(json.dumps(document))
            with self.assertRaisesRegex(ValueError, 'missing limits'):
                module.load_image_ports(path)

    def test_rust_declarations_are_cross_checks_and_never_support_authority(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base = root / 'crates/crysirror-formats-arc/src'
            base.mkdir(parents=True)
            native = base / 'images_a.rs'
            native.write_text('pub const FORMATS: &[&str] = &["fixture/image"];\n')
            registration = '''pub fn image_format_ids() -> Vec<&'static str> {crate::images_a::FORMATS.to_vec()}
            pub fn register_images(registry: &mut Registry) -> Result<()> {
                for &id in crate::images_a::FORMATS {
                    registry.register_format(Arc::new(Plugin {id,probe:crate::images_a::probe,decode:crate::images_a::decode}))?;
                } Ok(())
            }'''
            (base / 'images.rs').write_text(registration)
            ports = {'fixture/image': {'kind': 'image', 'registration_group': 'a', 'implementation': ['src/images_a.rs']}}
            module.validate_image_registration(ports, root)
            native.write_text(native.read_text() + '// pub const FORMATS: &[&str] = &["fixture/template"];\n')
            module.validate_image_registration(ports, root)
            native.write_text('pub const FORMATS: &[&str] = &["fixture/image", "fixture/pending"];')
            with self.assertRaisesRegex(ValueError, 'differs from native FORMATS'):
                module.validate_image_registration(ports, root)
            self.assertEqual(set(ports), {'fixture/image'})
            native.write_text('pub const FORMATS: &[&str] = &["fixture/image"];')
            (base / 'images.rs').write_text(registration.replace('pub fn register_images', 'pub fn unused_template') + '\npub fn register_images() {Ok(())}')
            with self.assertRaisesRegex(ValueError, 'not registered'):
                module.validate_image_registration(ports, root)

    def test_manifest_unknown_state_or_duplicate_does_not_claim_implementation(self):
        import json
        document = json.loads(module.IMAGE_MANIFEST.read_text())
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'support.json'
            document['images'][0]['status'] = 'pending'
            path.write_text(json.dumps(document))
            with self.assertRaisesRegex(ValueError, 'explicitly be ported or partial'):
                module.load_image_ports(path)
            document['images'][0]['status'] = 'ported'
            document['images'][1] = document['images'][0]
            path.write_text(json.dumps(document))
            with self.assertRaisesRegex(ValueError, 'duplicate image support'):
                module.load_image_ports(path)


if __name__ == "__main__":
    unittest.main()
