import { ButtonLink } from '~/components/ui/button';

export default function Room() {
  return (
    <div className="prose w-full min-h-dvh h-full flex items-center justify-center mx-auto">
      <div className="flex flex-col items-center justify-center">
        <h1 className="text-brand">Lobby</h1>
        <ButtonLink className="no-underline" variant="outline" href="/">
          Join a new room
        </ButtonLink>
      </div>
    </div>
  );
}
