"""MP-08 / MP-11: the helper files a local drill must copy, derived from what the helpers reference."""
import re
from pathlib import Path

SEEDS = ['native-x11.py', 'browser-desktop-protection.py', 'browser-protection-regions.mjs', 'browser-controller-frames.mjs', 'kernel-browser-refusal.mjs', 'kernel-browser-timing.mjs', 'native-keyboard-channel.mjs', 'browser-controller-snapshot.mjs', 'linux-owned-desktop.mjs', 'desktop-applications.mjs', 'linux-owned-process.mjs', 'linux-desktop-session.py', 'native-computer.mjs', 'native-computer.py', 'native-clipboard.py', 'slice-keyboard.py', 'slice-text-finder.py', 'x11-text-keyboard.py', 'x11-keymap-fence.py', 'browser-controller-actions.mjs', 'kernel-browser-native-tab-state.mjs', 'kernel-browser-native-worker.mjs', 'kernel-browser-policy.mjs', 'kernel-browser-tab-groups.mjs', 'native-accessibility.mjs', 'native-accessibility.py', 'room-native-protection.py']


def references(docker, name):
    """Sibling helpers named by file name or by native-computer.py's load('stem')."""
    text = (docker / name).read_text()
    stems = {match + '.py' for match in re.findall(r"load\('([\w-]+)'\)", text)}
    return {other.name for other in docker.iterdir() if other.suffix in ('.mjs', '.py') and other.name != name
            and (other.name in stems or re.search(r'(?<![\w.-])' + re.escape(other.name) + r'(?![\w-])', text))}


def default_helpers(docker):
    docker = Path(docker)
    found, pending = set(), [name for name in SEEDS if (docker / name).exists()]
    while pending:
        name = pending.pop()
        if name not in found:
            found.add(name)
            pending.extend(references(docker, name))
    return sorted(found)
