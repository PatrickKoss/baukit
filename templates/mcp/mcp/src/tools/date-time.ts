import { z } from 'zod';

export const dateTimeInput = z.union([
  z.iso.datetime({ offset: true }),
  z.iso.datetime({ offset: true, precision: -1 }),
]);
