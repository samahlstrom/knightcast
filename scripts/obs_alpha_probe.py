#!/usr/bin/env python3
"""Explicit isolated OBS alpha probe: temporary unselected scene, then cleanup.

Requires local OBS WebSocket, ffmpeg, websockets, numpy, and cv2. Refuses while
streaming or recording. Never selects a scene or changes an existing input.
Uses generated color blocks, not camera/video recordings. Saves tiny source
screenshots in /tmp for verifying alpha, then removes its own source and scene.
"""
import argparse
import asyncio
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import threading
import time
import uuid

import cv2
import numpy as np
import websockets


async def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--nonlinear-alpha', action='store_true', help='Probe the legacy gamma-space alpha option instead.')
    linear = not cli.parse_args().nonlinear_alpha
    config = json.loads((Path.home()/'.config/obs-studio/plugin_config/obs-websocket/config.json').read_text())
    async with websockets.connect(f'ws://127.0.0.1:{config["server_port"]}', max_size=16*1024*1024) as ws:
        hello = json.loads(await ws.recv())['d']
        identify = {'rpcVersion': 1, 'eventSubscriptions': 0}
        if 'authentication' in hello:
            auth = hello['authentication']
            secret = base64.b64encode(hashlib.sha256((config['server_password']+auth['salt']).encode()).digest()).decode()
            identify['authentication'] = base64.b64encode(hashlib.sha256((secret+auth['challenge']).encode()).digest()).decode()
        await ws.send(json.dumps({'op': 1, 'd': identify}))
        assert json.loads(await ws.recv())['op'] == 2

        async def request(kind, data=None):
            request_id = str(uuid.uuid4())
            await ws.send(json.dumps({'op': 6, 'd': {'requestType': kind, 'requestId': request_id, 'requestData': data or {}}}))
            while True:
                message = json.loads(await asyncio.wait_for(ws.recv(), 10))
                if message.get('op') == 7 and message['d']['requestId'] == request_id:
                    response = message['d']
                    if not response['requestStatus']['result']:
                        raise RuntimeError(f'{kind}: {response["requestStatus"]}')
                    return response.get('responseData', {})

        for kind in ['GetStreamStatus', 'GetRecordStatus']:
            if (await request(kind))['outputActive']:
                raise RuntimeError('Refusing probe while streaming or recording.')
        program = await request('GetCurrentProgramScene')
        before_scenes = {scene['sceneName'] for scene in (await request('GetSceneList'))['scenes']}
        before_inputs = {item['inputName'] for item in (await request('GetInputList'))['inputs']}
        tag = uuid.uuid4().hex[:10]
        scene, source = f'KnightCast alpha probe {tag}', f'KnightCast RGBA probe {tag}'
        frame = np.zeros((16, 64, 4), dtype=np.uint8)
        colors = [(255, 0, 0, 0), (255, 0, 0, 64), (255, 0, 0, 128), (0, 255, 0, 255)]
        for index, color in enumerate(colors):
            frame[:, index*16:(index+1)*16] = color
        command = ['ffmpeg', '-hide_banner', '-loglevel', 'error', '-nostdin', '-f', 'rawvideo',
                   '-pixel_format', 'rgba', '-video_size', '64x16', '-framerate', '60', '-i', 'pipe:0',
                   '-an', '-c:v', 'copy', '-f', 'nut', '-flush_packets', '1',
                   'tcp://127.0.0.1:8796?listen=1&tcp_nodelay=1']
        muxer = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        stop = threading.Event()
        def feed():
            try:
                while not stop.is_set():
                    muxer.stdin.write(frame.tobytes()); muxer.stdin.flush()
                    stop.wait(1/60)
            except (BrokenPipeError, OSError):
                pass
        feeder = threading.Thread(target=feed, daemon=True)
        feeder.start()
        created_scene = created_source = False
        samples = {}
        try:
            await asyncio.sleep(.3)
            if muxer.poll() is not None:
                raise RuntimeError('Probe muxer failed before OBS connection: '+muxer.stderr.read().decode())
            await request('CreateScene', {'sceneName': scene}); created_scene = True
            settings = {'is_local_file': False, 'input': 'tcp://127.0.0.1:8796', 'input_format': 'nut',
                        'buffering_mb': 0, 'reconnect_delay_sec': 1, 'hw_decode': False,
                        'linear_alpha': linear, 'restart_on_activate': False, 'close_when_inactive': False,
                        'clear_on_media_end': True, 'seekable': False,
                        'ffmpeg_options': 'probesize=32 analyzeduration=0'}
            await request('CreateInput', {'sceneName': scene, 'inputName': source, 'inputKind': 'ffmpeg_source',
                                         'inputSettings': settings, 'sceneItemEnabled': False})
            created_source = True
            await asyncio.sleep(1)
            for linear in [linear]:
                for attempt in range(30):
                    try:
                        screenshot = await request('GetSourceScreenshot', {'sourceName': source, 'imageFormat': 'png'})
                        png = base64.b64decode(screenshot['imageData'].split(',', 1)[1])
                        image = cv2.imdecode(np.frombuffer(png, dtype=np.uint8), cv2.IMREAD_UNCHANGED)
                        if image is not None and image.shape == (16, 64, 4) and image[8, 56, 3] > 0:
                            path = Path(f'/tmp/knightcast-obs-alpha-probe-{linear}.png')
                            path.write_bytes(png)
                            samples[str(linear)] = [image[8, x, [2, 1, 0, 3]].tolist() for x in [8, 24, 40, 56]]
                            break
                    except RuntimeError:
                        pass
                    await asyncio.sleep(.1)
                else:
                    raise RuntimeError(f'OBS did not decode probe pixels; media status={await request("GetMediaInputStatus", {"inputName": source})}')
            assert (await request('GetCurrentProgramScene')) == program, 'Program scene changed during probe'
            print(json.dumps({'input_rgba': colors, 'obs_source_screenshot_rgba_by_linear_alpha': samples}))
        finally:
            cleanup_errors = []
            if created_source:
                try:
                    await request('RemoveInput', {'inputName': source})
                except Exception as error:
                    cleanup_errors.append(str(error))
            if created_scene:
                try:
                    await request('RemoveScene', {'sceneName': scene})
                except Exception as error:
                    cleanup_errors.append(str(error))
            stop.set()
            muxer.terminate()
            try:
                muxer.wait(timeout=2)
            except subprocess.TimeoutExpired:
                muxer.kill(); muxer.wait()
            feeder.join(timeout=2)
            try:
                muxer.stdin.close()
            except OSError:
                pass
            muxer.stderr.close()
            if cleanup_errors:
                raise RuntimeError('OBS cleanup needs attention: '+ '; '.join(cleanup_errors))
            after_scenes = {item['sceneName'] for item in (await request('GetSceneList'))['scenes']}
            after_inputs = {item['inputName'] for item in (await request('GetInputList'))['inputs']}
            assert after_scenes == before_scenes and after_inputs == before_inputs, 'Unexpected OBS scene/source change'
            assert (await request('GetCurrentProgramScene')) == program
            print('Temporary probe source and scene removed; existing scene/source names and program scene unchanged.')


if __name__ == '__main__':
    asyncio.run(main())
