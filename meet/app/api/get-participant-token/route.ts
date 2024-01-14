import { NextRequest, NextResponse } from 'next/server';
import {
  AccessToken,
  AccessTokenOptions,
  CreateOptions,
  RoomServiceClient,
  VideoGrant,
} from 'livekit-server-sdk';
import { TokenResult } from '~/utils/types';

const apiKey = process.env.LIVEKIT_API_KEY;
const apiSecret = process.env.LIVEKIT_API_SECRET;
const livekitWsUrl = process.env.LIVEKIT_WS_URL;

const createToken = (userInfo: AccessTokenOptions, grant: VideoGrant) => {
  const at = new AccessToken(apiKey, apiSecret, userInfo);
  at.ttl = '5m';
  at.addGrant(grant);
  return at.toJwt();
};

const roomPattern = /\w{4}\-\w{4}\-\w{4}/;

export async function GET(req: NextRequest) {
  const room = req.nextUrl.searchParams.get('roomName');
  const identity = req.nextUrl.searchParams.get('identity');
  const name = req.nextUrl.searchParams.get('name');
  const metadata = req.nextUrl.searchParams.get('metadata');

  if (!room) {
    return NextResponse.json({ error: 'Missing "room" query parameter' }, { status: 400 });
  } else if (!name) {
    return NextResponse.json({ error: 'Missing "name" query parameter' }, { status: 400 });
  } else if (!identity) {
    return NextResponse.json({ error: 'Missing "identity" query parameter' }, { status: 400 });
  } else if (Array.isArray(metadata)) {
    return NextResponse.json({ error: 'provide max one metadata string' }, { status: 400 });
  }

  // enforce room name to be xxxx-xxxx-xxxx
  // this is simple & naive way to prevent user from guessing room names
  // please use your own authentication mechanisms in your own app
  if (!room.match(roomPattern)) {
    return NextResponse.json(
      { error: 'room name must match this format xxxx-xxxx' },
      { status: 400 },
    );
  }

  if (!livekitWsUrl) {
    return NextResponse.json({ error: 'Livekit Websocket Url not provided' }, { status: 400 });
  }

  if (!apiKey || !apiSecret) {
    return NextResponse.json({ error: 'Server misconfigured' }, { status: 500 });
  }

  const hostUrl = livekitWsUrl.replace('wss', 'https');
  const roomService = new RoomServiceClient(hostUrl, apiKey, apiSecret);
  await createRoomIfItDoesntExist(roomService, room);

  const grant: VideoGrant = {
    room,
    roomJoin: true,
    canPublish: true,
    canPublishData: true,
    canSubscribe: true,
  };

  const token = createToken({ identity, name, metadata: metadata ?? undefined }, grant);
  const result: TokenResult = {
    identity,
    accessToken: token,
  };

  return NextResponse.json({ ...result });
}

const createRoomIfItDoesntExist = async (roomService: RoomServiceClient, roomName: string) => {
  const opts: CreateOptions = {
    name: roomName,
    emptyTimeout: 5 * 60, // 5 minutes
    maxParticipants: 2,
  };

  let roomsWithSimilarNames = await roomService.listRooms([roomName]);
  if (!roomsWithSimilarNames.length) {
    const roomInfo = await roomService.createRoom(opts);
    if (roomInfo) {
      console.log('🎉room created - ', roomInfo.name);
    }
  }
};
