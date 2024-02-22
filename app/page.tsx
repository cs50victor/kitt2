'use client';
import { generateRandomAlphanumeric } from '~/lib/util';
import { LiveKitRoom, RoomAudioRenderer, StartAudio, useToken } from '@livekit/components-react';
import { AnimatePresence, motion } from 'framer-motion';
import { useCallback, useEffect, useMemo, useState } from 'react';

import Playground, { PlaygroundMeta, PlaygroundOutputs } from '~/components/Playground';
import { PlaygroundToast, ToastType } from '~/components/PlaygroundToast';
import { useAppConfig } from '~/hooks/useAppConfig';

const themeColors = ['cyan', 'green', 'amber', 'blue', 'violet', 'rose', 'pink', 'teal'];

export default function Page() {
  const [toastMessage, setToastMessage] = useState<{
    message: string;
    type: ToastType;
  } | null>(null);
  const [shouldConnect, setShouldConnect] = useState(false);
  const [liveKitUrl, setLiveKitUrl] = useState(process.env.NEXT_PUBLIC_LIVEKIT_URL);
  const [metadata, setMetadata] = useState<PlaygroundMeta[]>([]);

  const [roomName, setRoomName] = useState(createRoomName());

  const tokenOptions = useMemo(() => {
    return {
      userInfo: { identity: generateRandomAlphanumeric(16) },
    };
  }, []);

  // set a new room name each time the user disconnects so that a new token gets fetched behind the scenes for a different room
  useEffect(() => {
    if (shouldConnect === false) {
      setRoomName(createRoomName());
    }
  }, [shouldConnect]);

  useEffect(() => {
    const md: PlaygroundMeta[] = [];
    if (liveKitUrl && liveKitUrl !== process.env.NEXT_PUBLIC_LIVEKIT_URL) {
      md.push({ name: 'LiveKit URL', value: liveKitUrl });
    }
    if (tokenOptions.userInfo?.identity) {
      md.push({ name: 'Room Name', value: roomName });
      md.push({
        name: 'Participant Identity',
        value: tokenOptions.userInfo.identity,
      });
    }
    setMetadata(md);
  }, [liveKitUrl, roomName, tokenOptions]);

  const token = useToken('/api/token', roomName, tokenOptions);
  const appConfig = useAppConfig();
  const outputs = [
    appConfig?.outputs.audio && PlaygroundOutputs.Audio,
    appConfig?.outputs.video && PlaygroundOutputs.Video,
    appConfig?.outputs.chat && PlaygroundOutputs.Chat,
  ].filter((item) => typeof item !== 'boolean') as PlaygroundOutputs[];

  const handleConnect = useCallback((connect: boolean, opts?: { url: string; token: string }) => {
    if (connect && opts) {
      setLiveKitUrl(opts.url);
    }
    setShouldConnect(connect);
  }, []);

  return (
    <main className="relative flex flex-col justify-center px-4 items-center h-full w-full bg-black repeating-square-background">
      <AnimatePresence>
        {toastMessage && (
          <motion.div
            className="left-0 right-0 top-0 absolute z-10"
            initial={{ opacity: 0, translateY: -50 }}
            animate={{ opacity: 1, translateY: 0 }}
            exit={{ opacity: 0, translateY: -50 }}
          >
            <PlaygroundToast
              message={toastMessage.message}
              type={toastMessage.type}
              onDismiss={() => {
                setToastMessage(null);
              }}
            />
          </motion.div>
        )}
      </AnimatePresence>
      <LiveKitRoom
        className="flex flex-col h-full w-full"
        serverUrl={liveKitUrl}
        token={token}
        audio={appConfig?.inputs.mic}
        video={appConfig?.inputs.camera}
        connect={shouldConnect}
        onError={(e) => {
          setToastMessage({ message: e.message, type: 'error' });
          console.error(e);
        }}
      >
        <Playground
          title={appConfig?.title}
          githubLink={appConfig?.github_link}
          outputs={outputs}
          showQR={appConfig?.show_qr}
          themeColors={themeColors}
          defaultColor={appConfig?.theme_color ?? 'cyan'}
          onConnect={handleConnect}
          metadata={metadata}
          videoFit={appConfig?.video_fit ?? 'cover'}
        />
        <RoomAudioRenderer />
        <StartAudio label="Click to enable audio playback" />
      </LiveKitRoom>
    </main>
  );
}

function createRoomName() {
  return [generateRandomAlphanumeric(4), generateRandomAlphanumeric(4)].join('-');
}
