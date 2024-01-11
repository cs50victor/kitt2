import aspectRatioPlugin from '@tailwindcss/aspect-ratio';
import typographyPlugin from '@tailwindcss/typography';
import { shadcnPlugin } from './utils/shadcnPlugin';
import headlessui from '@headlessui/tailwindcss';
import animatePlugin from 'tailwindcss-animate';
import formsPlugin from '@tailwindcss/forms';
import { type Config } from 'tailwindcss';

export default {
  content: ['./{app,components}/**/*.{ts,tsx}'],
  plugins: [
    headlessui,
    animatePlugin,
    aspectRatioPlugin,
    typographyPlugin,
    formsPlugin,
    shadcnPlugin,
  ],
} satisfies Config;
