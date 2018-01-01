#!/usr/bin/env python3
"""websocket cmd client for wssrv.py example."""
import argparse
import asyncio
import signal
import sys

import aiohttp


def start_client(loop, url):
    name = input('Please enter your name: ')

    # send request
    ws = yield from aiohttp.ClientSession().ws_connect(url, autoclose=False, autoping=False)

    # input reader
    def stdin_callback():
        line = sys.stdin.buffer.readline().decode('utf-8')
        if not line:
            loop.stop()
        else:
            ws.send_str(name + ': ' + line)
    loop.add_reader(sys.stdin.fileno(), stdin_callback)

    @asyncio.coroutine
    def dispatch():
        while True:
            msg = yield from ws.receive()

            if msg.type == aiohttp.WSMsgType.TEXT:
                print('Text: ', msg.data.strip())
            elif msg.type == aiohttp.WSMsgType.BINARY:
                print('Binary: ', msg.data)
            elif msg.type == aiohttp.WSMsgType.PING:
                ws.pong()
            elif msg.type == aiohttp.WSMsgType.PONG:
                print('Pong received')
            else:
                if msg.type == aiohttp.WSMsgType.CLOSE:
                    yield from ws.close()
                elif msg.type == aiohttp.WSMsgType.ERROR:
                    print('Error during receive %s' % ws.exception())
                elif msg.type == aiohttp.WSMsgType.CLOSED:
                    pass

                break

    yield from dispatch()


ARGS = argparse.ArgumentParser(
    description="websocket console client for wssrv.py example.")
ARGS.add_argument(
    '--host', action="store", dest='host',
    default='127.0.0.1', help='Host name')
ARGS.add_argument(
    '--port', action="store", dest='port',
    default=8080, type=int, help='Port number')

if __name__ == '__main__':
    args = ARGS.parse_args()
    if ':' in args.host:
        args.host, port = args.host.split(':', 1)
        args.port = int(port)

    url = 'http://{}:{}/ws/'.format(args.host, args.port)

    loop = asyncio.get_event_loop()
    loop.add_signal_handler(signal.SIGINT, loop.stop)
    asyncio.Task(start_client(loop, url))
    loop.run_forever()
