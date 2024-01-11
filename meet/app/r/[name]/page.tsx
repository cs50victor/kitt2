'use client';
import '@livekit/components-styles';
import '@livekit/components-styles/prefabs';

import { decodePassphrase, encodePassphrase, randomString } from '~/utils/lksdk';
import { LocalUserChoices } from '@livekit/components-react';
import { ActiveRoom } from '~/components/ActiveRoom';
import dynamic from 'next/dynamic';
import { useState } from 'react';

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
    <main data-lk-theme="default" className="h-screen">
      {roomName && !Array.isArray(roomName) && preJoinChoices ? (
        <ActiveRoom roomName={roomName} userChoices={preJoinChoices} />
      ) : (
        <div className="flex flex-col justify-center items-center h-full">
          <h1 className="text-4xl font-semibold text-foreground font-display mb-4">Kitt2</h1>
          <PreJoinNoSSR
            className="!bg-foreground p-4 rounded-xl"
            onError={(err) => console.log('error while setting up prejoin', err)}
            defaults={{
              username: 'human',
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
