import type { Metadata } from 'next';
import { Inter, Roboto } from 'next/font/google';

import { TailwindIndicator } from '~/components/TailwindIndicator';
import { tw } from '~/utils/tw';

import './global.css';

const inter = Inter({
  subsets: ['latin'],
  variable: '--font-sans',
});

const display = Roboto({
  weight: '400',
  subsets: ['latin'],
  variable: '--font-display',
});

export const metadata: Metadata = {
  metadataBase: new URL(`https://${process.env.VERCEL_URL}`),
  title: {
    default: 'Livekit x Kitt2',
    template: '%s - Livekit Kitt2',
  },
  description:
    'LiveKit is an open source WebRTC project that gives you everything needed to build scalable and real-time audio and/or video experiences in your applications.',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html
      lang="en"
      className={tw('font-display h-full min-h-dvh antialiased', inter.variable, display.variable)}
    >
      <body className="w-full h-full">
        {children}
        <TailwindIndicator />
      </body>
    </html>
  );
}
