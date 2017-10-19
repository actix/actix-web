import asyncio
import aiohttp


def req1():
    with aiohttp.MultipartWriter() as writer:
        writer.append('test')
        writer.append_json({'passed': True})

    resp = yield from aiohttp.request(
        "post", 'http://localhost:8080/multipart',
        data=writer, headers=writer.headers)
    print(resp)
    assert 200 == resp.status


def req2():
    with aiohttp.MultipartWriter() as writer:
        writer.append('test')
        writer.append_json({'passed': True})
        writer.append(open('src/main.rs'))

    resp = yield from aiohttp.request(
        "post", 'http://localhost:8080/multipart',
        data=writer, headers=writer.headers)
    print(resp)
    assert 200 == resp.status


loop = asyncio.get_event_loop()
loop.run_until_complete(req1())
loop.run_until_complete(req2())
