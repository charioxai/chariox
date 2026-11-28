"""MP-08 / MP-10 / MP-11: public runner packages, outside official task source."""
import shlex


def setup_command(venv_root, codename):
    prefix = 'set -e; '
    apt = 'apt-get'
    if codename == 'bullseye':
        # Debian #1147093: live security indexes reference removed packages.
        # An isolated signed snapshot keeps the image's source lists unchanged.
        sources = venv_root + '-sources.list'
        snapshot = 'https://snapshot.debian.org/archive/debian-security/20260903T220410Z/'
        entries = ('deb https://deb.debian.org/debian bullseye main\n'
                   f'deb [check-valid-until=no] {snapshot} bullseye-security main\n')
        prefix += 'printf %s ' + shlex.quote(entries) + ' > ' + shlex.quote(sources) + '; '
        apt = shlex.join(['apt-get', '-o', 'Dir::Etc::sourcelist=' + sources,
                         '-o', 'Dir::Etc::sourceparts=-'])
    return (prefix +
        'if ! command -v git >/dev/null 2>&1 || ! test -f /usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf; then ' +
        apt + ' update -qq; ' + apt + ' install -y -qq git fonts-dejavu-core; fi; ' +
        'python3 -m venv ' + shlex.quote(venv_root) + ' 2>/dev/null || (' +
        apt + ' update -qq && ' + apt + ' install -y -qq python3 python3-venv && ' +
        'python3 -m venv ' + shlex.quote(venv_root) + '); ' +
        shlex.quote(venv_root + '/bin/pip') + ' install pyte==0.8.2 pillow==11.3.0')
