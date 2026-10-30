import hashlib
import json
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VM = ROOT / 'crates/crysirror-rfvp-vm'


class RfvpPolicyTest(unittest.TestCase):
    def test_first_class_sources_record_origin_and_local_changes(self):
        source = json.loads((VM / 'SOURCE.json').read_text())
        self.assertRegex(source['commit'], r'^[0-9a-f]{40}$')
        self.assertEqual(source['license'], 'MPL-2.0')
        for rel, digest in source['source_sha256'].items():
            current = hashlib.sha256((VM / rel).read_bytes()).hexdigest()
            patch = source['patched'].get(rel)
            if patch:
                self.assertEqual(patch['origin_sha256'], digest, rel)
                self.assertEqual(patch['current_sha256'], current, rel)
                self.assertTrue(patch['reason'], rel)
            else:
                self.assertEqual(current, digest, rel)
        for rel in source['removed_sources']:
            self.assertFalse((VM / rel).exists(), rel)

    def test_no_proprietary_fonts_vendored(self):
        fonts = [p for p in VM.rglob('*') if p.suffix.lower() in ('.ttf', '.ttc', '.otf')]
        self.assertEqual(fonts, [])

    def test_no_upstream_runtime_or_private_renderer_mixer(self):
        workspace = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']
        dep = workspace['dependencies']['crysirror-rfvp-vm']
        self.assertEqual(dep['path'], 'crates/crysirror-rfvp-vm')
        self.assertNotIn('rfvp', workspace['dependencies'])
        self.assertFalse((ROOT / 'crates/rfvp-upstream').exists())
        self.assertFalse((VM / 'src/host_api/soft_raster.rs').exists())
        audio = (VM / 'src/host_api/audio.rs').read_text()
        self.assertNotIn('SoftAudioMixer', audio)
        for package in ('crysirror-rfvp', 'crysirror-rfvp-vm'):
            manifest = tomllib.loads((ROOT / 'crates' / package / 'Cargo.toml').read_text())
            for forbidden in ('wgpu', 'winit', 'rodio', 'cpal', 'kira', 'ab_glyph', 'miniz_oxide'):
                self.assertNotIn(forbidden, manifest['dependencies'])

    def test_vm_globals_are_explicit_instance_state(self):
        global_source = (VM / 'src/script/global.rs').read_text()
        self.assertNotIn('static GLOBAL', global_source)
        self.assertNotIn('UnsafeCell', global_source)
        context = (VM / 'src/script/context.rs').read_text()
        self.assertIn('syscaller.globals()', context)
        self.assertIn('independent_native_opcode_owners_can_run_on_parallel_workers', context)
        self.assertIn('pub(crate) globals:', (VM / 'src/subsystem/world.rs').read_text())

    def test_native_policy_calls_shared_text_and_image_capabilities(self):
        text = (VM / 'src/subsystem/resources/text_manager.rs').read_text()
        self.assertIn('SlotTextBank', text)
        self.assertIn('readed_text', text)  # native read/save policy stays in the VM
        self.assertNotIn('fn draw_string_unit', text)
        self.assertNotIn('rasterize_config', (VM / 'src/font.rs').read_text())
        texture = (VM / 'src/subsystem/resources/texture.rs').read_text()
        graph = (VM / 'src/subsystem/resources/graph_buff.rs').read_text()
        self.assertIn('crysirror_image::nvsg::NvsgImage::decode', texture)
        self.assertIn('crysirror_image::pixels::to_rgba8', texture)
        self.assertIn('crysirror_image::associated::tone_channels_percent', graph)
        self.assertIn('crysirror_image::bitmap::blit_rgba8', graph)
        self.assertNotIn('fn apply_one', graph)
        shared = (ROOT / 'crates/crysirror-text/src/vn_slot.rs').read_text()
        for forbidden in ('crysirror_rfvp', 'crate::subsystem', 'crate::script'):
            self.assertNotIn(forbidden, shared)
        self.assertIn('shared_slot_bank_rasterizes_and_reveals_without_a_native_vm', shared)


if __name__ == '__main__':
    unittest.main()
