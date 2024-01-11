import { headers } from 'next/headers';

export const getBaseUrl = (getHeaders: typeof headers) => {
  const _headers = getHeaders();
  const domain = _headers.get('host');
  const protocol = domain?.startsWith('localhost:') ? 'http://' : 'https://';
  return `${protocol}${domain}`;
};

export const IS_PROD = process.env.NODE_ENV === 'production';
