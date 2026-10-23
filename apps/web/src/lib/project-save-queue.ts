const queues = new Map<string, Promise<unknown>>();

export function enqueueProjectSave<T>(projectId: string, save: () => Promise<T>): Promise<T> {
  const previous = queues.get(projectId) ?? Promise.resolve();
  let release!: () => void;
  const next = new Promise<void>((resolve) => { release = resolve; });
  queues.set(projectId, next);
  const run = previous.catch(() => undefined).then(save);
  void run.then(release, release).finally(() => {
    if (queues.get(projectId) === next) queues.delete(projectId);
  });
  return run;
}
