import { expect, test } from "vite-plus/test";
import { frameMailbox } from "@/lib/frame-mailbox";

test("snapshot bursts present only the latest sample on the next animation frame", () => {
  let callback: FrameRequestCallback | undefined;
  let scheduled = 0;
  const presented: number[] = [];
  const queue = frameMailbox<number>(
    (value) => presented.push(value),
    (next) => {
      callback = next;
      return ++scheduled;
    },
    () => {},
  );
  for (let value = 1; value <= 100; value++) queue.push(value);
  expect(presented).toEqual([]);
  expect(scheduled).toBe(1);
  callback!(0);
  expect(presented).toEqual([100]);
  queue.push(101);
  callback!(16);
  expect(presented).toEqual([100, 101]);
});

test("unmount cancels pending presentation and ignores late snapshots", () => {
  let callback: FrameRequestCallback | undefined;
  const presented: number[] = [];
  const cancelled: number[] = [];
  const queue = frameMailbox<number>(
    (value) => presented.push(value),
    (next) => {
      callback = next;
      return 7;
    },
    (id) => cancelled.push(id),
  );
  queue.push(1);
  queue.dispose();
  callback!(0);
  queue.push(2);
  expect(cancelled).toEqual([7]);
  expect(presented).toEqual([]);
});
