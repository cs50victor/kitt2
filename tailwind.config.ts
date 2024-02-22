import aspectRatioPlugin from '@tailwindcss/aspect-ratio';
import typographyPlugin from '@tailwindcss/typography';
import { shadcnPlugin } from './utils/shadcnPlugin';
import headlessui from '@headlessui/tailwindcss';
import animatePlugin from 'tailwindcss-animate';
import formsPlugin from '@tailwindcss/forms';
import { type Config } from 'tailwindcss';
import colors from 'tailwindcss/colors';

const shades = ['50', '100', '200', '300', '400', '500', '600', '700', '800', '900', '950'];
const colorList = [
  'gray',
  'green',
  'cyan',
  'amber',
  'violet',
  'blue',
  'rose',
  'pink',
  'teal',
  'red',
];
const uiElements = [
  'bg',
  'selection:bg',
  'border',
  'text',
  'hover:bg',
  'hover:border',
  'hover:text',
  'ring',
  'focus:ring',
];
const customColors = {
  cyan: colors.cyan,
  green: colors.green,
  amber: colors.amber,
  violet: colors.violet,
  blue: colors.blue,
  rose: colors.rose,
  pink: colors.pink,
  teal: colors.teal,
  red: colors.red,
};

let shadowNames = [];
let customShadows: Record<string, string> = {};
let textShadows: Record<string, string> = {};
let textShadowNames: Array<string> = [];

for (const [name, color] of Object.entries(customColors)) {
  customShadows[`${name}`] = `0px 0px 10px ${color['500']}`;
  customShadows[`lg-${name}`] = `0px 0px 20px ${color['600']}`;
  textShadows[`${name}`] = `0px 0px 4px ${color['700']}`;
  textShadowNames.push(`drop-shadow-${name}`);
  shadowNames.push(`shadow-${name}`);
  shadowNames.push(`shadow-lg-${name}`);
  shadowNames.push(`hover:shadow-${name}`);
}

const safelist = [
  'bg-black',
  'bg-white',
  'transparent',
  'object-cover',
  'object-contain',
  ...shadowNames,
  ...textShadowNames,
  ...shades.flatMap((shade) => [
    ...colorList.flatMap((color) => [
      ...uiElements.flatMap((element) => [`${element}-${color}-${shade}`]),
    ]),
  ]),
];

export default {
  content: ['./{app,components}/**/*.{ts,tsx}'],
  extend: {
    dropShadow: {
      ...textShadows,
    },
    boxShadow: {
      ...customShadows,
    },
  },
  safelist,
  plugins: [
    headlessui,
    animatePlugin,
    aspectRatioPlugin,
    typographyPlugin,
    formsPlugin,
    shadcnPlugin,
  ],
} satisfies Config;
