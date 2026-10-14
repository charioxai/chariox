"""MP-08/MP-10/MP-11: client and row scoped native keymap acknowledgements."""
import importlib.util
from pathlib import Path
import struct
from types import SimpleNamespace
import threading
import unittest

spec=importlib.util.spec_from_file_location('fence',Path(__file__).with_name('x11-keymap-fence.py'))
fence=importlib.util.module_from_spec(spec);spec.loader.exec_module(fence)


def xkb(rows, order='<'):
    first=min(rows);last=max(rows);data=bytearray(40)
    data[0]=1;data[17]=first;data[20]=last-first+1
    struct.pack_into(order+'H',data,12,2)
    for code in range(first,last+1):
        symbols=rows.get(code,[0]);header=bytearray(8)
        struct.pack_into(order+'H',header,6,len(symbols));data.extend(header)
        data.extend(struct.pack(order+'I'*len(symbols),*symbols))
    struct.pack_into(order+'I',data,4,(len(data)-32)//4)
    return bytes(data)


class Replies(unittest.TestCase):
    def test_only_the_recycled_row_can_acknowledge_unicode(self):
        wanted=0x01004e16
        for order in ['<','>']:
            data=xkb({8:[0x01005b57],92:[wanted]},order)
            self.assertFalse(fence.keymap_contains(data,order,(False,0,0),8,wanted))
            self.assertTrue(fence.keymap_contains(xkb({8:[wanted,wanted]},order),order,(False,0,0),8,wanted))

    def test_inconsistent_levels_and_truncated_maps_are_not_acknowledgements(self):
        wanted=0x01004e16
        self.assertFalse(fence.keymap_contains(xkb({8:[wanted,0x01005b57]}),'<',(False,0,0),8,wanted))
        data=xkb({8:[wanted]})
        for size in range(len(data)):
            self.assertFalse(fence.keymap_contains(data[:size],'<',(False,0,0),8,wanted))

    def test_core_reply_uses_its_correlated_request_range(self):
        wanted=0x01004e16;data=bytearray(32);data[0]=1;data[1]=2
        data.extend(struct.pack('<IIII',0,0,wanted,wanted));struct.pack_into('<I',data,4,4)
        self.assertTrue(fence.keymap_contains(data,'<',(True,7,2),8,wanted))
        self.assertFalse(fence.keymap_contains(data,'<',(True,8,2),8,wanted))
        self.assertFalse(fence.keymap_contains(data,'<',(True,9,2),8,wanted))

    def test_another_client_or_uncorrelated_reply_cannot_release_input(self):
        instance=fence.KeymapFence.__new__(fence.KeymapFence)
        instance.lock=threading.Lock();instance.ready=threading.Event();instance.received=threading.Event()
        instance.wanted=0x01004e16;instance.code=8;instance.failed=False
        instance.matched=[100];instance.requests={}
        data=bytearray(xkb({8:[instance.wanted]}));struct.pack_into('<H',data,2,12)
        reply=lambda client:SimpleNamespace(category=fence.record.FromServer,id_base=client,data=data,client_swapped=False)
        instance.reply(reply(101));instance.reply(reply(100));self.assertFalse(instance.received.is_set())
        instance.requests[(100,12)]=(False,0,0);instance.reply(reply(100));self.assertTrue(instance.received.is_set())

    def test_arm_retires_a_previous_acknowledgement(self):
        instance=fence.KeymapFence.__new__(fence.KeymapFence);instance.lock=threading.Lock();instance.received=threading.Event();instance.received.set()
        instance.arm(8,0x01004e16)
        self.assertFalse(instance.received.is_set());self.assertEqual(instance.code,8)


if __name__=='__main__':unittest.main()
