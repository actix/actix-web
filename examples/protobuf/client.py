# just start server and run client.py

# wget https://github.com/google/protobuf/releases/download/v3.5.1/protobuf-python-3.5.1.zip
# unzip protobuf-python-3.5.1.zip.1
# cd protobuf-3.5.1/python/
# python3.6 setup.py install

# pip3.6 install --upgrade pip
# pip3.6 install aiohttp

#!/usr/bin/env python
import test_pb2
import traceback
import sys

import asyncio
import aiohttp

def op():
    try:
        obj = test_pb2.MyObj()
        obj.number = 9
        obj.name = 'USB'

        #Serialize
        sendDataStr = obj.SerializeToString()
        #print serialized string value
        print('serialized string:', sendDataStr)
        #------------------------#
        #  message transmission  #
        #------------------------#
        receiveDataStr = sendDataStr
        receiveData = test_pb2.MyObj()

        #Deserialize
        receiveData.ParseFromString(receiveDataStr)
        print('pares serialize string, return: devId = ', receiveData.number, ', name = ', receiveData.name)
    except(Exception, e):
        print(Exception, ':', e)
        print(traceback.print_exc())
        errInfo = sys.exc_info()
        print(errInfo[0], ':', errInfo[1])


async def fetch(session):
    obj = test_pb2.MyObj()
    obj.number = 9
    obj.name = 'USB'
    async with session.post('http://localhost:8080/', data=obj.SerializeToString(),
        headers={"content-type": "application/protobuf"}) as resp:
        print(resp.status)
        data = await resp.read()
        receiveObj = test_pb2.MyObj()
        receiveObj.ParseFromString(data)
        print(receiveObj)

async def go(loop):
    obj = test_pb2.MyObj()
    obj.number = 9
    obj.name = 'USB'
    async with aiohttp.ClientSession(loop=loop) as session:
        await fetch(session)

loop = asyncio.get_event_loop()
loop.run_until_complete(go(loop))
loop.close()