import { NextResponse, type NextRequest } from 'next/server'
import { generateRoomId } from './lib/client-utils'

export async function middleware(request: NextRequest) {
  const url = request.nextUrl.clone()
  if (url.pathname === '/') {
    const roomUrl = new URL(`/r/${generateRoomId()}`, request.url);
    return NextResponse.redirect(roomUrl)
  }
}
