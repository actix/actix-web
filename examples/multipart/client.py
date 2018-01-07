# This script could be used for actix-web multipart example test
# just start server and run client.py

import asyncio
import aiohttp

async def req1():
    with aiohttp.MultipartWriter() as writer:
        writer.append('test')
        writer.append_json({'passed': True})

    resp = await aiohttp.ClientSession().request(
        "post", 'http://localhost:8080/multipart',
        data=writer, headers=writer.headers)
    print(resp)
    assert 200 == resp.status


async def req2():
    with aiohttp.MultipartWriter() as writer:
        writer.append('test')
        writer.append_json({'passed': True})
        writer.append(open('src/main.rs'))

    resp = await aiohttp.ClientSession().request(
        "post", 'http://localhost:8080/multipart',
        data=writer, headers=writer.headers)
    print(resp)
    assert 200 == resp.status


loop = asyncio.get_event_loop()
loop.run_until_complete(req1())
loop.run_until_complete(req2())
