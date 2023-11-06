import { NextApiRequest, NextApiResponse } from 'next';

import { AccessToken, RoomServiceClient, Room } from 'livekit-server-sdk';
import type { AccessTokenOptions, CreateOptions, VideoGrant } from 'livekit-server-sdk';
import { TokenResult } from '../../lib/types';
import { RoomOptions } from 'livekit-client';

const apiKey = process.env.LIVEKIT_API_KEY;
const apiSecret = process.env.LIVEKIT_API_SECRET;
const livekitWsUrl = process.env.LIVEKIT_URL;

const createToken = (userInfo: AccessTokenOptions, grant: VideoGrant) => {
  const at = new AccessToken(apiKey, apiSecret, userInfo);
  at.ttl = '5m';
  at.addGrant(grant);
  return at.toJwt();
};

const roomPattern = /\w{4}\-\w{4}/;

export default async function handleToken(req: NextApiRequest, res: NextApiResponse) {
  try {
    const { roomName, identity, name, metadata } = req.query;

    if (typeof identity !== 'string' || typeof roomName !== 'string') {
      res.status(403).end();
      return;
    }

    if (Array.isArray(name)) {
      throw Error('provide max one name');
    }
    if (Array.isArray(metadata)) {
      throw Error('provide max one metadata string');
    }

    // enforce room name to be xxxx-xxxx
    // this is simple & naive way to prevent user from guessing room names
    // please use your own authentication mechanisms in your own app
    if (!roomName.match(roomPattern)) {
      res.status(400).end();
      return;
    }

    // if (!userSession.isAuthenticated) {
    //   res.status(403).end();
    //   return;
    // }
    if (!livekitWsUrl) {
      throw Error('Livekit Websocket Url not provided');
    }
    const hostUrl = livekitWsUrl.replace("wss", "https");
    const roomService = new RoomServiceClient(hostUrl, apiKey, apiSecret);
    await createRoomIfItDoesntExist(roomService, roomName);

    const grant: VideoGrant = {
      room: roomName,
      roomJoin: true,
      canPublish: true,
      canPublishData: true,
      canSubscribe: true,
    };

    const token = createToken({ identity, name, metadata }, grant);
    const result: TokenResult = {
      identity,
      accessToken: token,
    };

    res.status(200).json(result);
  } catch (e) {
    res.statusMessage = (e as Error).message;
    res.status(500).end();
  }
}


const createRoomIfItDoesntExist=async(roomService:RoomServiceClient, roomName: string)=>{
  const opts : CreateOptions = {
    name: roomName,
    emptyTimeout: 5 * 60, // 5 minutes
    maxParticipants: 2,
  };

  let roomsWithSimilarNames = await roomService.listRooms([roomName]);
  if(!roomsWithSimilarNames.length){
    const roomInfo = await roomService.createRoom(opts);
    if (roomInfo){
      console.log('🎉room created - ', roomInfo.name);
    };
  }
}
