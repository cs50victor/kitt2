'use client';
import {
  LiveKitRoom,
  LocalUserChoices,
  VideoConference,
  formatChatMessageLinks,
  useToken,
} from '@livekit/components-react';

import {
  ExternalE2EEKeyProvider,
  LogLevel,
  Room,
  RoomConnectOptions,
  RoomOptions,
  VideoCodec,
  VideoPresets,
} from 'livekit-client';

import { useRouter, useSearchParams } from 'next/navigation';
import { useMemo } from 'react';
import { decodePassphrase, useServerUrl } from '~/utils/lksdk';
import { DebugMode } from './Debug';

export type ActiveRoomProps = {
  userChoices: LocalUserChoices;
  roomName: string;
  region?: string;
};

export const ActiveRoom = ({ roomName, userChoices }: ActiveRoomProps) => {
  const router = useRouter();

  const token = useToken(process.env.NEXT_PUBLIC_LK_TOKEN_ENDPOINT, roomName, {
    userInfo: {
      identity: userChoices.username,
      name: userChoices.username,
    },
  });

  const searchParams = useSearchParams();
  const region = searchParams.get('region');
  const hq = searchParams.get('hq');
  const codec = searchParams.get('codec') ?? 'vp9';

  const liveKitUrl = useServerUrl(region as string | undefined);

  const worker =
    typeof window !== 'undefined' &&
    userChoices.e2ee &&
    new Worker(new URL('livekit-client/e2ee-worker', import.meta.url));

  const e2eeEnabled = !!(userChoices.e2ee && worker);
  const keyProvider = new ExternalE2EEKeyProvider();

  const roomOptions = useMemo((): RoomOptions => {
    return {
      videoCaptureDefaults: {
        deviceId: userChoices.videoDeviceId ?? undefined,
        resolution: hq === 'true' ? VideoPresets.h2160 : VideoPresets.h720,
      },
      publishDefaults: {
        dtx: false,
        videoSimulcastLayers:
          hq === 'true'
            ? [VideoPresets.h1080, VideoPresets.h720]
            : [VideoPresets.h540, VideoPresets.h216],
        red: !e2eeEnabled,
        videoCodec: codec as VideoCodec | undefined,
      },
      audioCaptureDefaults: {
        deviceId: userChoices.audioDeviceId ?? undefined,
      },
      adaptiveStream: { pixelDensity: 'screen' },
      dynacast: true,
      e2ee: e2eeEnabled
        ? {
            keyProvider,
            worker,
          }
        : undefined,
    };
  }, [userChoices, hq, codec]);

  const room = useMemo(() => new Room(roomOptions), []);

  if (e2eeEnabled) {
    keyProvider.setKey(decodePassphrase(userChoices.sharedPassphrase));
    room.setE2EEEnabled(true);
  }
  const connectOptions = useMemo((): RoomConnectOptions => {
    return {
      autoSubscribe: true,
    };
  }, []);

  return (
    <>
      {liveKitUrl && (
        <LiveKitRoom
          room={room}
          token={token}
          serverUrl={liveKitUrl}
          connectOptions={connectOptions}
          video={false}
          audio={userChoices.audioEnabled}
          onDisconnected={() => {
            console.log('leaving room');
            router.push('/r');
          }}
        >
          <VideoConference chatMessageFormatter={formatChatMessageLinks} />
          <DebugMode logLevel={LogLevel.info} />
        </LiveKitRoom>
      )}
    </>
  );
};
