#!/usr/bin/env python3
"""Adversarial and end-to-end acceptance against a real log-print/2 Core process."""
import json
import socket
import time
import unittest
import uuid
from support import Core, RPC, RemoteError, eventually


class ProtocolTests(unittest.TestCase):
    def test_unique_ids_receive_order_and_no_saved_semantics(self):
        with Core() as c, c.rpc('a') as a, c.rpc('b') as b, c.rpc() as admin:
            raw, other = c.stream('a'), c.stream('b')
            uuid.UUID(raw)
            self.assertNotEqual(raw, other)
            for i in [9,3,7]:
                a.publish(raw,str(i).encode(), source_seq=i)
            b.publish(other,b'independent')
            page = admin.call('read',stream=raw)
            self.assertEqual([r['source_seq'] for r in page['records']],[9,3,7])
            self.assertEqual([r['seq'] for r in page['records']],[1,2,3])
            self.assertTrue(all('durability' not in r for r in page['records']))
            self.assertEqual(admin.call('read',stream=other)['records'][0]['payload'],list(b'independent'))
            self.assertEqual(admin.call('stream.get',stream=raw)['description'],'stream A')

    def test_rollover_record_limit_and_bytes_are_independent_per_stream(self):
        for options in [{'buffer_records':3},{'buffer_bytes':1024,'max_payload_bytes':1024}]:
            with self.subTest(options=options), Core(options=options) as c, c.rpc('a') as a, c.rpc('b') as b, c.rpc() as admin:
                raw, other = c.stream('a'), c.stream('b')
                b.publish(other,b'keep-me')
                for i in range(20):
                    a.publish(raw,bytes([i])*200)
                records = admin.call('read',stream=raw)['records']
                self.assertTrue(records)
                self.assertLess(len(records),20)
                self.assertEqual(records[-1]['seq'],20)
                self.assertEqual(admin.call('read',stream=other)['records'][0]['payload'],list(b'keep-me'))

    def test_independent_subscriptions_oldest_then_wait_then_continue(self):
        with Core(options={'buffer_records':3}) as c, c.rpc('a') as a:
            stream = c.stream('a')
            for i in range(5):
                a.publish(stream,str(i).encode())
            with c.rpc('out',events=True) as first, c.rpc('other-out',events=True) as second:
                first.call('subscribe',stream=stream)
                second.call('subscribe',stream=stream)
                for client in (first,second):
                    records = [client.receive()['record'] for _ in range(3)]
                    self.assertEqual([r['seq'] for r in records],[3,4,5])
                # No-data interval must not end subscriptions or the stream.
                time.sleep(.15)
                a.publish(stream,b'later')
                self.assertEqual(first.receive()['record']['payload'],list(b'later'))
                self.assertEqual(second.receive()['record']['payload'],list(b'later'))

    def test_empty_subscription_receives_future_data(self):
        with Core() as c, c.rpc('a') as a, c.rpc('out',events=True) as out:
            stream = c.stream('a')
            out.call('subscribe',stream=stream)
            time.sleep(.1)
            a.publish(stream,b'first')
            self.assertEqual(out.receive()['record']['seq'],1)

    def test_single_writer_and_foreign_stream_rejected(self):
        with Core() as c, c.rpc('a') as a, c.rpc('b') as b, c.rpc('out') as out:
            stream = c.stream('a')
            with self.assertRaises((RemoteError,EOFError)):
                with c.rpc('a'):
                    pass
            for client in (b,out):
                with self.assertRaises(RemoteError):
                    client.publish(stream,b'forbidden')
            a.publish(stream,b'owned')

    def test_owner_disconnect_keeps_memory_and_does_not_close_stream(self):
        with Core() as c:
            with c.rpc('a') as a:
                stream = c.stream('a')
                a.publish(stream,b'buffered')
            time.sleep(.1)
            with c.rpc() as admin, c.rpc('out',events=True) as out:
                self.assertEqual(admin.call('read',stream=stream)['records'][0]['payload'],list(b'buffered'))
                out.call('subscribe',stream=stream)
                self.assertEqual(out.receive()['record']['payload'],list(b'buffered'))

    def test_core_restart_has_no_history_and_new_identity(self):
        with Core() as c:
            with c.rpc('a') as a:
                original = c.stream('a')
                a.publish(original,b'not-persistent')
            c.stop()
            c.start()
            with c.rpc('a') as a, c.rpc() as admin:
                current = c.stream('a')
                self.assertNotEqual(current,original)
                self.assertEqual(admin.call('read',stream=current)['records'],[])
                with self.assertRaises(RemoteError):
                    admin.call('stream.get',stream=original)
            self.assertFalse(list(c.path.rglob('*.sqlite*')))
            self.assertFalse(list(c.path.rglob('*.db')))

    def test_wrong_token_old_protocol_and_unknown_identity_rejected(self):
        with Core() as c:
            for kwargs in [dict(plugin='a',token='wrong'),dict(plugin='unknown',token='a'),dict(plugin='a',token='a',protocol='log-print/1')]:
                with self.subTest(kwargs=kwargs), self.assertRaises((RemoteError,EOFError)):
                    with RPC(c.address,**kwargs):
                        pass
            with c.rpc() as admin:
                self.assertIn('streams',admin.call('status'))

    def test_invalid_publish_has_no_partial_record(self):
        with Core(options={'max_payload_bytes':100}) as c, c.rpc('a') as a, c.rpc() as admin:
            stream = c.stream('a')
            for payload in [[256],list(b'x'*101),'not-bytes']:
                with self.assertRaises(RemoteError):
                    a.call('publish',stream=stream,key='bad',payload=payload)
            self.assertEqual(admin.call('read',stream=stream)['records'],[])
            a.publish(stream,b'ok')
            self.assertEqual(admin.call('read',stream=stream)['records'][0]['seq'],1)

    def test_unread_slow_subscriber_does_not_block_other_stream_or_input(self):
        with Core(options={'buffer_records':8}) as c, c.rpc('a') as a, c.rpc('b') as b, c.rpc('out',events=True) as slow, c.rpc() as admin:
            raw, other = c.stream('a'),c.stream('b')
            slow.call('subscribe',stream=raw)
            before = time.monotonic()
            for i in range(300):
                a.publish(raw,b'x'*8192,key=str(i))
            b.publish(other,b'still-responsive')
            self.assertLess(time.monotonic()-before,12)
            self.assertEqual(admin.call('read',stream=other)['records'][0]['payload'],list(b'still-responsive'))
            self.assertLessEqual(len(admin.call('read',stream=raw)['records']),8)

    def test_invalid_frame_does_not_take_down_core(self):
        with Core() as c:
            host,port = c.address.split(':')
            for payload in [b'{invalid}\n',b'x'*(1024*1024+1)]:
                with socket.create_connection((host,int(port)),timeout=2) as sock:
                    try:
                        sock.sendall(payload)
                        sock.settimeout(6)
                        self.assertFalse(sock.recv(1024))
                    except (ConnectionResetError,BrokenPipeError):
                        pass
            with c.rpc() as admin:
                self.assertIn('streams',admin.call('status'))

    def test_udp_registration_publish_no_ack_and_subscription(self):
        with Core(transport='udp') as c, c.rpc('a') as a, c.rpc() as admin, c.rpc('out',events=True) as out:
            stream = c.stream('a')
            self.assertEqual(a.welcome['protocol'],'log-print/2')
            out.call('subscribe',stream=stream)
            a.publish(stream,b'udp',source_seq=42)
            received = out.receive()['record']
            self.assertEqual(received['payload'],list(b'udp'))
            self.assertEqual(received['source_seq'],42)
            # Publish success must not produce a per-record response.
            a.socket.settimeout(.15)
            with self.assertRaises(socket.timeout):
                a.receive()
            self.assertEqual(admin.call('read',stream=stream)['records'][0]['payload'],list(b'udp'))

    def test_udp_bad_registration_does_not_claim_writer(self):
        with Core(transport='udp') as c:
            with self.assertRaises((RemoteError,EOFError)):
                RPC(c.address,'a','wrong','udp')
            with c.rpc('a') as a:
                self.assertEqual(a.welcome['protocol'],'log-print/2')


if __name__ == '__main__':
    unittest.main(verbosity=2)
