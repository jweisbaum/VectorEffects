/**
 * Builds plane meshes off the main thread.
 *
 * A mesh with the map's outline or a pole in it is half a second to two and a
 * half of inverse projection. On the main thread that was a frozen window
 * each time the pointer came to rest; here the map goes on being drawn through
 * the mesh it has, and swaps when this answers.
 */
import { buildPlaneMeshData, transferablesOf, type PlaneMeshRequest } from "./planeMesh";

const scope = self as unknown as {
  onmessage: ((event: MessageEvent<{ id: number; request: PlaneMeshRequest }>) => void) | null;
  postMessage(message: unknown, transfer: Transferable[]): void;
};

scope.onmessage = (event) => {
  const { id, request } = event.data;
  try {
    const data = buildPlaneMeshData(request);
    scope.postMessage({ id, data }, transferablesOf(data));
  } catch (error) {
    scope.postMessage({ id, error: String(error) }, []);
  }
};
