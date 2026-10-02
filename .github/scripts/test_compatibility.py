"""Checks for release completeness and useful compatibility failure diagnostics."""

import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


packaging = load('package-compatibility')
reporting = load('report-compatibility')
versions = load('build-version')


class Versions(unittest.TestCase):
    def setUp(self):
        day = patch.object(versions, 'build_date', return_value='20261002')
        day.start()
        self.addCleanup(day.stop)

    def fixture(self, root):
        (root / 'src-tauri').mkdir()
        (root / 'src-tauri/tauri.conf.json').write_text('{"version": "0.1.0-beta"}\n')
        (root / 'version.txt').write_text('0.1.0-beta.1+20261002\n')

    def test_counter_increments_for_multiple_builds_on_the_same_day(self):
        self.assertEqual(versions.next_version('0.1.0-beta.1+20261002', '0.1.0-beta'), '0.1.0-beta.2+20261002')
        self.assertEqual(versions.next_version('0.1.0-beta.999+20261002', '0.1.0-beta'), '0.1.0-beta.1000+20261002')

    def test_new_day_updates_date_without_resetting_counter(self):
        self.assertEqual(versions.next_version('0.1.0-beta.2+20261001', '0.1.0-beta'),
                         '0.1.0-beta.3+20261002')

    def test_invalid_counter_or_date_is_rejected(self):
        for version in ('0.1.0-beta.0+20261002', '0.1.0-beta.01+20261002',
                        '0.1.0-beta.1+20260230', '0.1.0-beta.1+2026102'):
            with self.subTest(version=version), self.assertRaises(ValueError):
                versions.counter(version, '0.1.0-beta')

    def test_display_version_does_not_change_package_version(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            versions.stamp(root, '0.1.0-beta.2+20261002')
            self.assertEqual((root / 'version.txt').read_text(), '0.1.0-beta.2+20261002\n')
            self.assertEqual(versions.package_version(root), '0.1.0-beta')
            with self.assertRaises(ValueError):
                versions.stamp(root, '0.2.0-beta.3+20261002')
            self.assertEqual((root / 'version.txt').read_text(), '0.1.0-beta.2+20261002\n')

    def test_main_reservation_retries_conflict_with_updated_counter(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            first = {'encoding': 'base64', 'sha': 'old',
                     'content': base64.b64encode(b'0.1.0-beta.1+20261002\n').decode()}
            second = {'encoding': 'base64', 'sha': 'new',
                      'content': base64.b64encode(b'0.1.0-beta.2+20261002\n').decode()}
            with patch.object(versions, 'gh', side_effect=[first, versions.GitHubError('HTTP 409'), second, {}]) as api:
                version = versions.reserve(root, 'citadel-foss/portal', 'refs/heads/main')
            self.assertEqual(version, '0.1.0-beta.3+20261002')
            self.assertEqual((root / 'version.txt').read_text(), version + '\n')
            payload = api.call_args.kwargs['payload']
            self.assertEqual(payload['sha'], 'new')
            self.assertEqual(payload['branch'], 'main')
            self.assertEqual(base64.b64decode(payload['content']), b'0.1.0-beta.3+20261002\n')

    def test_development_branch_does_not_write_repository_counter(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            with patch.object(versions, 'gh') as api:
                version = versions.reserve(root, 'citadel-foss/portal', 'refs/heads/dev')
            api.assert_not_called()
            self.assertEqual(version, '0.1.0-beta.2+20261002')


class Downloads(unittest.TestCase):
    def fixtures(self, root):
        for artifact, extension in [('portal-linux-amd64-appimage', '.AppImage'),
                                    ('portal-linux-amd64-deb', '.deb'),
                                    ('portal-linux-x86_64-rpm', '.rpm'),
                                    ('portal-mac-universal-dmg', '.dmg'),
                                    ('portal-mac-universal-app-archive', '.app.tar.gz')]:
            directory = root / artifact
            directory.mkdir(parents=True)
            (directory / ('Portal_0.1.0-beta' + extension)).write_bytes(artifact.encode())
        for asset, _ in packaging.SERVERS:
            directory = root / ('portal-server-' + asset)
            directory.mkdir()
            binary = directory / 'portal-server'
            binary.write_bytes(asset.encode())
            binary.chmod(0o644)

    def test_all_downloads_have_stable_names_and_executable_servers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixtures(root / 'downloads')
            output = root / 'release'
            assets = packaging.package(root / 'downloads', output)
            self.assertEqual(len(assets), 8)
            for asset, _ in packaging.SERVERS:
                archive_name = 'portal-server-' + asset.replace('mac-', 'macos-') + '.tar.gz'
                with tarfile.open(output / archive_name) as archive:
                    binary = archive.getmember('portal-server')
                    self.assertEqual(binary.mode, 0o755)
                    self.assertEqual(archive.extractfile(binary).read(), asset.encode())
            notes = packaging.describe(output, assets, 'citadel-foss/portal', '0.1.0-beta',
                                       'a' * 40, 'b' * 40, 'https://example.com/run/1', '0.1.0-beta', '0.1.0-beta.2+20261002')
            manifest = json.loads((output / 'build.json').read_text())
            self.assertEqual(manifest['openswap_sha'], 'a' * 40)
            self.assertEqual(manifest['portal_sha'], 'b' * 40)
            self.assertEqual(manifest['portal_version'], '0.1.0-beta')
            self.assertEqual(manifest['display_version'], '0.1.0-beta.2+20261002')
            self.assertIn('0.1.0-beta.2+20261002', notes)
            self.assertEqual(len(manifest['assets']), 8)
            for entry in manifest['assets']:
                self.assertIn('/releases/download/0.1.0-beta/' + entry['name'], entry['url'])
                self.assertEqual(entry['sha256'], hashlib.sha256((output / entry['name']).read_bytes()).hexdigest())
                self.assertIn(entry['url'], notes)
            checksums = (output / 'SHA256SUMS').read_text().splitlines()
            self.assertEqual(len(checksums), 9)
            for line in checksums:
                checksum, name = line.split('  ')
                self.assertEqual(checksum, hashlib.sha256((output / name).read_bytes()).hexdigest())

    def test_missing_platform_aborts_before_producing_release_assets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixtures(root / 'downloads')
            (root / 'downloads/portal-server-linux-arm64/portal-server').unlink()
            with self.assertRaisesRegex(RuntimeError, 'linux-arm64'):
                packaging.package(root / 'downloads', root / 'release')
            self.assertFalse((root / 'release').exists())

    def test_duplicate_installer_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixtures(root / 'downloads')
            (root / 'downloads/portal-linux-amd64-deb/another.deb').write_bytes(b'duplicate')
            with self.assertRaisesRegex(RuntimeError, 'found 2'):
                packaging.package(root / 'downloads', root / 'release')


class Diagnostics(unittest.TestCase):
    def test_rust_error_keeps_source_location_and_removes_color_and_timestamps(self):
        summary, excerpt = reporting.error_excerpt(
            '2026-10-02T12:00:00Z Compiling dependencies\n'
            '2026-10-02T12:00:01Z \x1b[31merror[E0599]: no method named receive\x1b[0m\n'
            '2026-10-02T12:00:01Z   --> core/src/wallet.rs:42:5\n'
        )
        self.assertEqual(summary, 'error[E0599]: no method named receive')
        self.assertIn('core/src/wallet.rs:42:5', excerpt)
        self.assertNotIn('Compiling dependencies', excerpt)
        self.assertNotIn('\x1b', excerpt)
        self.assertNotIn('2026-', excerpt)

    def test_typescript_error_is_not_replaced_by_generic_exit_status(self):
        summary, excerpt = reporting.error_excerpt(
            'src/App.tsx(12,4): error TS2322: Type mismatch\n'
            '##[error]Process completed with exit code 2.\n'
        )
        self.assertIn('TS2322', summary)
        self.assertIn('src/App.tsx(12,4)', excerpt)

    def test_unknown_failure_keeps_the_log_tail_and_escapes_markdown_fences(self):
        _, excerpt = reporting.error_excerpt('Preparing build\nSomething broke ```\n')
        self.assertIn('Something broke', excerpt)
        self.assertNotIn('```', excerpt)

    def test_generic_exit_status_keeps_preceding_unrecognized_error(self):
        summary, excerpt = reporting.error_excerpt(
            'Preparing installer\n'
            'failed to bundle application: missing resource\n'
            '##[error]Process completed with exit code 1.\n'
        )
        self.assertIn('see the job log', summary)
        self.assertIn('missing resource', excerpt)

    def run_report(self, existing):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            commands = []
            job = {'id': 5, 'conclusion': 'failure', 'name': 'Desktop Linux x86_64',
                   'html_url': 'https://example.com/job/5',
                   'steps': [{'name': 'Build installers', 'conclusion': 'failure'}]}

            def fake_gh(*args):
                commands.append(args)
                endpoint = args[-1]
                if args[0] == 'api' and '/attempts/2/jobs?' in endpoint:
                    return json.dumps([{'jobs': [job]}])
                if args[0] == 'api' and endpoint.endswith('/jobs/5/logs'):
                    return 'error[E0599]: upstream API changed\n  --> core/src/ops.rs:8:2\n'
                if args[0] == 'api' and '/issues?' in endpoint:
                    return json.dumps([[{'number': 7, 'body': '<!-- openswap-compatibility:' + 'a' * 40 + ' -->',
                                         'html_url': 'https://example.com/issues/7'}]] if existing else [[]])
                if args[:2] == ('issue', 'create'):
                    return 'https://example.com/issues/8\n'
                return ''

            env = {'GITHUB_REPOSITORY': 'citadel-foss/portal', 'GITHUB_RUN_ID': '1',
                   'GITHUB_RUN_ATTEMPT': '2', 'OPENSWAP_SHA': 'a' * 40,
                   'PORTAL_SHA': 'b' * 40, 'GITHUB_STEP_SUMMARY': str(root / 'summary.md')}
            previous = Path.cwd()
            try:
                os.chdir(root)
                with patch.dict(os.environ, env), patch.object(reporting, 'gh', side_effect=fake_gh):
                    reporting.report()
                body = (root / 'compatibility-report/issue.md').read_text()
                self.assertIn('Desktop Linux x86_64', body)
                self.assertIn('Build installers', body)
                self.assertIn('error[E0599]', body)
                self.assertIn('core/src/ops.rs:8:2', body)
                self.assertTrue((root / 'compatibility-report/job-5.log').exists())
            finally:
                os.chdir(previous)
            return commands

    def test_failure_creates_issue_with_diagnostic_and_failed_platform(self):
        commands = self.run_report(existing=False)
        self.assertEqual(sum(command[:2] == ('issue', 'create') for command in commands), 1)
        self.assertFalse(any(command[:2] == ('issue', 'edit') for command in commands))

    def test_repeat_failure_updates_existing_issue_instead_of_creating_duplicate(self):
        commands = self.run_report(existing=True)
        self.assertTrue(any(command[:3] == ('issue', 'edit', '7') for command in commands))
        self.assertTrue(any(command[:3] == ('issue', 'comment', '7') for command in commands))
        self.assertFalse(any(command[:2] == ('issue', 'create') for command in commands))


if __name__ == '__main__':
    unittest.main()
