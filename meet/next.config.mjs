import { config } from "dotenv"

config({ path: `${process.env.ENVIRONMENT ?? "."}` });

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // experimental: {
  //   ppr: true
  // },
};

export default nextConfig;
