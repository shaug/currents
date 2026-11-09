"""Integration regressions for the reviewed documentation package (stdlib only)."""
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]
RETIRED = ('docs/implementation-plan.md', 'docs/planning/publication.md',
           'docs/planning/publication-map.json')


class PlanningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = pathlib.Path(self.temp.name)
        shutil.copytree(REPO / 'docs', self.repo / 'docs')
        shutil.copyfile(REPO / 'README.md', self.repo / 'README.md')

    def run_script(self, name):
        return subprocess.run([sys.executable, str(self.repo / 'docs/planning' / name)],
                              cwd=self.repo, capture_output=True, text=True)

    def assert_success(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def bodies(self):
        return {p.name: p.read_bytes() for p in
                (self.repo / 'docs/planning/issues').glob('*.md')}

    def test_regeneration_without_retired_documents_never_recreates_them(self):
        for name in RETIRED:
            (self.repo / name).unlink(missing_ok=True)
        original = (self.repo / 'docs/planning/manifest.json').read_bytes()
        self.assert_success(self.run_script('author_payloads.py'))
        self.assertEqual((self.repo / 'docs/planning/manifest.json').read_bytes(), original)
        self.assert_success(self.run_script('render.py'))
        self.assertEqual(len(self.bodies()), 67)
        for name in RETIRED:
            self.assertFalse((self.repo / name).exists(), name)
        self.assert_success(self.run_script('validate.py'))

    def test_rendering_is_deterministic_and_preserves_all_acceptance_assertions(self):
        self.assert_success(self.run_script('render.py'))
        first = self.bodies()
        self.assert_success(self.run_script('render.py'))
        self.assertEqual(first, self.bodies())
        manifest = json.loads((self.repo / 'docs/planning/manifest.json').read_text())
        self.assertEqual(sum(len(t['tests']) for t in manifest['tickets']), 255)
        for ticket in manifest['tickets']:
            body = first[ticket['id'] + '.md'].decode()
            for test in ticket['tests']:
                self.assertIn(f"- [ ] `{test['name']}`: {test['assertion']}\n", body)
            self.assertIn(ticket['verification']['command'], body)

    def test_every_body_links_durable_guidance(self):
        self.assert_success(self.run_script('render.py'))
        for name, body in self.bodies().items():
            self.assertIn(b'(../../engineering-guide.md)', body, name)
            self.assertNotIn(b'implementation-plan.md', body, name)

    def test_validator_catches_broken_links_in_current_docs(self):
        for name in ('README.md', 'docs/review-record.md', 'docs/engineering-guide.md'):
            with self.subTest(name=name):
                path = self.repo / name
                before = path.read_text() if path.exists() else '# Engineering guide\n'
                path.write_text(before + '\n[Missing](missing-current-document.md)\n')
                result = self.run_script('validate.py')
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn('Broken local link', result.stdout)
                path.write_text(before)

    def test_validator_rejects_retired_links_even_if_target_exists(self):
        for name in RETIRED:
            with self.subTest(name=name):
                target = self.repo / name
                target.write_text('Historical content\n')
                readme = self.repo / 'README.md'
                before = readme.read_text()
                readme.write_text(before + f'\n[Retired]({name})\n')
                result = self.run_script('validate.py')
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn('Retired document dependency', result.stdout)
                readme.write_text(before)

    def test_validator_rejects_mutable_github_links_to_retired_documents(self):
        readme = self.repo / 'README.md'
        before = readme.read_text()
        prefixes = ('https://github.com/shaug/currents/blob/',
                    'https://github.com/shaug/currents/raw/',
                    'https://raw.githubusercontent.com/shaug/currents/')
        for prefix in prefixes:
            for ref in ('main', 'HEAD', 'release/v1', 'refs/heads/main', '67811de'):
                for name in RETIRED:
                    url = f'{prefix}{ref}/{name}?raw=1#history'
                    with self.subTest(url=url):
                        readme.write_text(before + f'\n[Retired]({url})\n')
                        result = self.run_script('validate.py')
                        self.assertNotEqual(result.returncode, 0, result.stdout)
                        self.assertIn('Retired document dependency', result.stdout)
        readme.write_text(before)

    def test_validator_allows_historical_commit_links_and_unrelated_urls(self):
        readme = self.repo / 'README.md'
        before = readme.read_text()
        commit = '67811de85c685278c5543b965967e47127b1f0c0'
        prefixes = ('https://github.com/shaug/currents/blob/',
                    'https://github.com/shaug/currents/raw/',
                    'https://raw.githubusercontent.com/shaug/currents/')
        urls = [f'{prefix}{commit}/{name}' for prefix in prefixes for name in RETIRED]
        urls += ['https://github.com/other/currents/blob/main/docs/implementation-plan.md',
                 'https://github.com/shaug/currents/blob/main/docs/engineering-guide.md']
        for url in urls:
            with self.subTest(url=url):
                readme.write_text(before + f'\n[Reference]({url})\n')
                self.assert_success(self.run_script('validate.py'))
        readme.write_text(before)

    def test_validator_detects_body_drift_without_rewriting_it(self):
        for name in ('CUR-001.md', 'E01.md'):
            with self.subTest(name=name):
                path = self.repo / 'docs/planning/issues' / name
                before = path.read_text()
                path.write_text(before + '\nUnreviewed body drift.\n')
                result = self.run_script('validate.py')
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn('Rendered body differs', result.stdout)
                self.assertTrue(path.read_text().endswith('Unreviewed body drift.\n'))
                path.write_text(before)


if __name__ == '__main__':
    unittest.main()
