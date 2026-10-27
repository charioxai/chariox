"""MP-08 / MP-10 / MP-11: keep preflight state/evidence outside source."""
from pathlib import Path


def require_external(path):
    root = Path(__file__).resolve().parents[5]
    if not path.is_absolute() or path.resolve().is_relative_to(root):
        raise ValueError('MP-10 absolute external state/evidence path required')
    return path
