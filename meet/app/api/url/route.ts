import { NextRequest, NextResponse } from 'next/server';
import { getLiveKitURL } from '~/utils/server-utils';

export async function GET(req: NextRequest) {
  const region = req.nextUrl.searchParams.get('region');

  if (Array.isArray(region)) {
    return NextResponse.json({ error: 'provide max one region string' }, { status: 400 });
  }

  try {
    const url = getLiveKitURL(region ?? undefined);
    console.log(`region: ${region}, url: ${url}`);
    return NextResponse.json({ url });
  } catch (e) {
    console.error('---->', e);
    return NextResponse.json({ error: (e as Error).message }, { status: 500 });
  }
}
