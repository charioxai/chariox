# MP-08/MP-10: fail closed before copying an unpinned Node runtime.
import tempfile
import unittest
from pathlib import Path
from lan_node_runtime import install
class OfficialNode(unittest.TestCase):
    def test_unpinned_archive_never_creates_binary(self):
        with tempfile.TemporaryDirectory(prefix='chariox-display-node-') as root:
            archive=Path(root)/'node.tar.xz';archive.write_bytes(b'distro runtime')
            dest=Path(root)/'bin/node'
            with self.assertRaisesRegex(ValueError,'checksum mismatch'):install(archive,dest)
            self.assertFalse(dest.exists())
if __name__=='__main__':unittest.main()
