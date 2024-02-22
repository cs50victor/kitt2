'use client';
import '@livekit/components-styles';
import '@livekit/components-styles/prefabs';

import { decodePassphrase, encodePassphrase, randomString } from '~/utils/lksdk';
import { LocalUserChoices } from '@livekit/components-react';
import { ActiveRoom } from '~/components/ActiveRoom';
import dynamic from 'next/dynamic';
import { useState } from 'react';

// Required to access localStorage
const PreJoinNoSSR = dynamic(
  async () => {
    return (await import('@livekit/components-react')).PreJoin;
  },
  { ssr: false },
);

export default function LivekitRoom({ params }: { params: { name: string } }) {
  const roomName = params.name;

  const e2eePassphrase =
    typeof window !== 'undefined' && decodePassphrase(location.hash.substring(1));

  const [preJoinChoices, setPreJoinChoices] = useState<LocalUserChoices | undefined>(undefined);

  function handlePreJoinSubmit(values: LocalUserChoices) {
    if (values.e2ee) {
      location.hash = encodePassphrase(values.sharedPassphrase);
    }
    setPreJoinChoices(values);
  }

  return (
    <main data-lk-theme="default">
      {roomName && !Array.isArray(roomName) && preJoinChoices ? (
        <div className="h-screen">
          <ActiveRoom roomName={roomName} userChoices={preJoinChoices} />
        </div>
      ) : (
        <div className="flex flex-col justify-center items-center h-full min-h-dvh">
          {/* <h1 className="text-4xl font-semibold font-display text-brand">Kitt2</h1> */}
          <PreJoinNoSSR
            className="p-4 rounded-xl w-full max-w-sm
            [&*.lk-camera-off-note]:bg-cyan-950
            [&*.lk-camera-off-note]:flex
            [&*.lk-camera-off-note]:items-center
            [&*.lk-camera-off-note]:justify-center
            [&*.lk-camera-off-note>svg]:stroke-black
            [&*.lk-camera-off-note>svg>path]:fill-gray-300
            [&*.lk-camera-off-note]:rounded-full
            [&*.lk-camera-off-note]:aspect-w-2
            [&*.lk-camera-off-note]:aspect-h-2
            [&*.lk-camera-off-note]:overflow-hidden
            [&*.lk-camera-off-note]:border

            [&*.lk-button-group-container]:flex
            [&*.lk-button-group-container]:flex-col
            [&*.lk-button-group-container]:my-8
            [&*.lk-button-group-container]:space-y-4
            
            [&*.lk-username-container]:flex
            [&*.lk-username-container]:flex-col
            [&*.lk-username-container]:my-8
            [&*.lk-username-container]:space-y-4
            
            [&*.lk-button-group>button]:w-full
            "
            onError={(err) => console.log('error while setting up prejoin', err)}
            defaults={{
              username: 'Pinkman',
              videoEnabled: false,
              audioEnabled: false,
              e2ee: true,
              sharedPassphrase: e2eePassphrase || randomString(64),
            }}
            onSubmit={handlePreJoinSubmit}
          />
        </div>
      )}
    </main>
  );
}
