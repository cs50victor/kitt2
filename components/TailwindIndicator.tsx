import { IS_PROD } from '~/utils/helpers';

export const TailwindIndicator = () => {
  if (IS_PROD) return null;

  return (
    <div className="fixed bottom-0 left-0 z-50 flex h-6 w-6 items-center justify-center rounded-r bg-foreground px-1 text-xs text-background">
      <div className="block sm:hidden">xs</div>
      <div className="hidden sm:block md:hidden">sm</div>
      <div className="hidden md:block lg:hidden">md</div>
      <div className="hidden lg:block xl:hidden">lg</div>
      <div className="hidden xl:block 2xl:hidden">xl</div>
      <div className="hidden 2xl:block">2xl</div>
    </div>
  );
};
