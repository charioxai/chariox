import { addDays, differenceInDays, format, parseISO, isWeekend, startOfMonth } from 'date-fns';

export function run() {
  const a = parseISO('2026-02-27T10:30:00');
  const b = addDays(a, 3);
  return [
    format(a, 'yyyy-MM-dd HH:mm EEEE'),
    format(b, 'do MMMM yyyy'),
    differenceInDays(parseISO('2026-12-25'), parseISO('2026-10-22')),
    isWeekend(b),
    format(startOfMonth(b), 'yyyy-MM-dd'),
  ].join(' | ');
}
