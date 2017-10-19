import asyncio
import aiohttp


def client():
    with aiohttp.MultipartWriter() as writer:
        writer.append('test')
        writer.append_json({'passed': True})

    resp = yield from aiohttp.request(
        "post", 'http://localhost:8080/multipart',
        data=writer, headers=writer.headers)
    print(resp)
    assert 200 == resp.status


loop = asyncio.get_event_loop()
loop.run_until_complete(client())
