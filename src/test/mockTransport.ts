// Test double for the IPC port. Install it with `setTransport(...)` in a
// `beforeEach`; nothing auto-installs a transport, so a test that forgets
// fails loudly instead of reaching a runtime that is not there.

import type {
  IpcTransport,
  OpenPathOptions,
  SavePathOptions,
} from "../ipc/transport";

export type RecordedInvoke = {
  command: string;
  payload: Record<string, unknown> | undefined;
};

export type MockTransport = IpcTransport & {
  /// Every `invoke` in call order.
  calls: RecordedInvoke[];
  /// Payload of the single call to `command`; throws when the count is
  /// not exactly one, so a silent extra dispatch cannot pass unnoticed.
  payloadOf: (command: string) => Record<string, unknown> | undefined;
  /// Make `command` resolve with `value` instead of `undefined`.
  reply: (command: string, value: unknown) => void;
  /// Make `command` reject with `error`.
  failWith: (command: string, error: Error) => void;
  /// What the next `pickOpenPath` / `pickSavePath` resolves to.
  nextOpenPath: string | null;
  nextSavePath: string | null;
  openCalls: OpenPathOptions[];
  saveCalls: SavePathOptions[];
};

export const createMockTransport = (): MockTransport => {
  const replies = new Map<string, unknown>();
  const failures = new Map<string, Error>();

  const mock: MockTransport = {
    calls: [],
    openCalls: [],
    saveCalls: [],
    nextOpenPath: null,
    nextSavePath: null,

    invoke: <T,>(
      command: string,
      payload?: Record<string, unknown>,
    ): Promise<T> => {
      mock.calls.push({ command, payload });
      const failure = failures.get(command);
      if (failure) return Promise.reject(failure);
      return Promise.resolve(replies.get(command) as T);
    },

    pickOpenPath: (options: OpenPathOptions) => {
      mock.openCalls.push(options);
      return Promise.resolve(mock.nextOpenPath);
    },

    pickSavePath: (options: SavePathOptions) => {
      mock.saveCalls.push(options);
      return Promise.resolve(mock.nextSavePath);
    },

    payloadOf: (command: string) => {
      const matches = mock.calls.filter((c) => c.command === command);
      if (matches.length !== 1) {
        throw new Error(
          `[RAIL test] expected exactly one "${command}" call, saw ${matches.length}`,
        );
      }
      return matches[0].payload;
    },

    reply: (command: string, value: unknown) => {
      replies.set(command, value);
    },

    failWith: (command: string, error: Error) => {
      failures.set(command, error);
    },
  };

  return mock;
};
