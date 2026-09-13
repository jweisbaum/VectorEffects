import { warmTargets, type StepState } from "./playback";

export interface PreparationRequest {
  step: number;
  first: number;
  last: number;
  loop: boolean;
  rate: number;
  states: readonly StepState[];
}
export interface PreparationStatus { ready: number; total: number; streaming: boolean }

/** Texture slots bound both preparation and protection, including the held frame. */
export function preparationTargets(
  request: PreparationRequest,
  tiles: number,
  capacity: number,
  latencyMs: number,
): { targets: number[]; protected: number[]; streaming: boolean } {
  const slots = Math.max(1, Math.floor(capacity / Math.max(1, tiles)) - 2);
  const count = request.last - request.first + 1;
  const streaming = count > slots;
  const depth = Math.min(slots - 1, Math.max(2, Math.ceil(request.rate * latencyMs * 2 / 1000)));
  const protectedSteps = [request.step, ...warmTargets(request.step, request.last, request.loop, request.first, depth)];
  return {
    targets: streaming ? protectedSteps : [request.step, ...warmTargets(request.step, request.last, true, request.first, count - 1)],
    protected: protectedSteps,
    streaming,
  };
}

export interface PlaybackMap {
  prepare(request: PreparationRequest): PreparationStatus;
  /** Draw a resident step synchronously, before publishing its playhead position. */
  present(step: number): boolean;
}
