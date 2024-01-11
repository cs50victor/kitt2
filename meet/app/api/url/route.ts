import { NextRequest, NextResponse } from 'next/server';
import { getLiveKitURL } from '~/utils/server';

export async function GET(req: NextRequest) {
  const region = req.nextUrl.searchParams.get('region');

  if (Array.isArray(region)) {
    return NextResponse.json({ error: 'provide max one region string' }, { status: 400 });
  }

  try {
    const url = getLiveKitURL(region ?? undefined);
    return NextResponse.json({ url });
  } catch (e) {
    console.error('Couldnt fetch livekit url : ', e);
    return NextResponse.json({ error: (e as Error).message }, { status: 500 });
  }
}
