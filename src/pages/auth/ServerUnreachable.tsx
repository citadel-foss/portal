import { Button } from "../../components/ui/inputs";
import { Card } from "../../components/ui/display";
import { IntroStage } from "../../components/ui/IntroStage";

/**
 * Shown when the API cannot be reached at all — distinct from being signed out. A password
 * prompt here would be a dead end: there is nothing on the other side to check it.
 */
export function ServerUnreachable({ onRetry }: { onRetry: () => void }) {
  return (
    <IntroStage
      lead="Can't reach"
      accent="Portal"
      caption="The app is running, but its backend isn't answering"
      instant
      className="min-h-screen"
    >
      <div className="mx-auto w-full max-w-lg">
        <Card className="hairline border-line-strong">
          <div className="flex flex-col gap-4 p-8 text-left">
            <p className="text-[13px] leading-6 text-muted">
              This page loaded, but nothing is serving Portal's API. In development that
              usually means only the UI half started — <code className="text-foreground">npm
              run web:dev</code> runs both, and prints an error if the backend could not take
              the data directory because another Portal already has it.
            </p>
            <Button onClick={onRetry}>Try again</Button>
          </div>
        </Card>
      </div>
    </IntroStage>
  );
}
