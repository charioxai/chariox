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


def ping(order, nonce=77):
    data=bytearray(44);data[0]=25;data[12]=fence.X.ClientMessage;data[13]=32
    struct.pack_into(order+'H',data,2,11)
    struct.pack_into(order+'I',data,20,55)
    struct.pack_into(order+'III',data,24,66,nonce,1000)
    return bytes(data)


def client_buffer(order, messages, client=100):
    # RECORD element headers use the recording connection's byte order,
    # independently of the recorded client's wire order. Gaps are legitimate:
    # unselected requests also increment the client's sequence number.
    data=b''.join(struct.pack('=I',seq)+raw for seq,raw in messages)
    return SimpleNamespace(category=fence.record.FromClient,id_base=client,data=data,
        element_header=fence.record.FromClientSequence,recorded_sequence_number=messages[0][0],
        client_swapped=order!=('<' if fence.sys.byteorder=='little' else '>'))


def instance():
    result=fence.KeymapFence.__new__(fence.KeymapFence)
    result.lock=threading.Lock();result.ready=threading.Event();result.received=threading.Event()
    result.ping_received=threading.Event();result.ping_nonce=77
    result.protocol_atom=55;result.ping_atom=66;result.window=SimpleNamespace(id=1000)
    result.focus_client=100;result.matched=[100,200];result.requests={};result.wanted=None
    result.major=135;result.failed=False
    return result


class Replies(unittest.TestCase):
    def test_delivery_requires_the_focused_client_to_echo_the_current_ping(self):
        result=instance()
        for order in ['<','>']:
            for synthetic in [0,128]:
                data=bytearray(ping(order));data[12]|=synthetic
                def submit(raw,client=100):
                    result.ping_received.clear()
                    result.reply(client_buffer(order,[(12,bytes(raw))],client))
                    return result.ping_received.is_set()
                self.assertFalse(submit(data,200),'a companion connection is not the UI acknowledgement')
                for offset in [20,24,28,32]:
                    changed=bytearray(data);changed[offset]^=1
                    self.assertFalse(submit(changed))
                for size in range(44):self.assertFalse(submit(data[:size]))
                self.assertTrue(submit(data))

    def test_batched_send_events_find_the_current_echo_after_stale_or_unrelated_echo(self):
        for order in ['<','>']:
            current=ping(order);unrelated=bytearray(current);unrelated[24]^=1
            for earlier in [ping(order,76),bytes(unrelated)]:
                result=instance()
                result.reply(client_buffer(order,[(12,earlier),(15,current)]))
                self.assertTrue(result.ping_received.is_set())
                self.assertEqual(result.requests,{})
                other=instance();other.reply(client_buffer(order,[(12,earlier),(15,current)],200))
                self.assertFalse(other.ping_received.is_set())

    def test_batched_map_requests_keep_each_elements_sequence_with_gaps_and_wrap(self):
        for order in ['<','>']:
            result=instance();result.wanted=0x01004e16;result.code=8
            core=bytearray(8);core[0]=101;core[4]=8;core[5]=1
            struct.pack_into(order+'H',core,2,2)
            xkb_request=bytearray(28);xkb_request[0]=result.major;xkb_request[1]=8
            struct.pack_into(order+'H',xkb_request,2,7)
            result.reply(client_buffer(order,[(65534,bytes(core)),(65537,bytes(xkb_request))]))
            self.assertEqual(result.requests,{(100,65534):(True,8,1),(100,1):(False,0,0)})
            stale=bytearray(xkb({8:[0x01005b57]},order));struct.pack_into(order+'H',stale,2,65534)
            valid=bytearray(xkb({8:[result.wanted]},order));struct.pack_into(order+'H',valid,2,1)
            result.reply(SimpleNamespace(category=fence.record.FromServer,id_base=100,
                data=bytes(stale)+bytes(valid),element_header=fence.record.FromClientSequence,
                client_swapped=order!=('<' if fence.sys.byteorder=='little' else '>')))
            self.assertTrue(result.received.is_set());self.assertEqual(result.requests,{})

    def test_malformed_batched_request_cannot_acknowledge_or_stall_the_parser(self):
        for order in ['<','>']:
            current=ping(order)
            for size in range(48):
                result=instance();value=client_buffer(order,[(12,current)])
                value.data=value.data[:size];result.reply(value)
                self.assertFalse(result.ping_received.is_set())
            for length in [0,1,10,12,65535]:
                invalid=bytearray(current);struct.pack_into(order+'H',invalid,2,length)
                result=instance();result.reply(client_buffer(order,[(12,bytes(invalid)),(15,current)]))
                self.assertFalse(result.ping_received.is_set())

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
        reply=lambda client:SimpleNamespace(category=fence.record.FromServer,id_base=client,data=data,client_swapped=False,element_header=fence.record.FromClientSequence)
        instance.reply(reply(101));instance.reply(reply(100));self.assertFalse(instance.received.is_set())
        instance.requests[(100,12)]=(False,0,0);instance.reply(reply(100));self.assertTrue(instance.received.is_set())

    def test_arm_retires_a_previous_acknowledgement(self):
        instance=fence.KeymapFence.__new__(fence.KeymapFence);instance.lock=threading.Lock();instance.received=threading.Event();instance.received.set()
        instance.arm(8,0x01004e16)
        self.assertFalse(instance.received.is_set());self.assertEqual(instance.code,8)


if __name__=='__main__':unittest.main()
