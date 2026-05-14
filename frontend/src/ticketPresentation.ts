import { type TicketModel } from "./SyncEngine";
import { getLocaleConfig, translate } from "./i18n";

export interface SavedLocationRecord {
  id: string;
  label: string;
  location: any;
}

export type SavedLocationIndex = Record<string, SavedLocationRecord>;

const LEGACY_WEEKDAY_LABELS: Array<{ token: string; dayIndex: number }> = [
  { token: "monday", dayIndex: 1 },
  { token: "tuesday", dayIndex: 2 },
  { token: "wednesday", dayIndex: 3 },
  { token: "thursday", dayIndex: 4 },
  { token: "friday", dayIndex: 5 },
  { token: "saturday", dayIndex: 6 },
  { token: "sunday", dayIndex: 0 },
];

type RRuleFreq = "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY";

interface ParsedRRule {
  freq: RRuleFreq | null;
  byDay: string[] | null;
  byMonthDay: number[] | null;
  byMonth: number[] | null;
  until: string | null;
  count: number | null;
  interval: number;
}

function parseRRuleComponent(rrulePart: string): ParsedRRule {
  const result: ParsedRRule = {
    freq: null,
    byDay: null,
    byMonthDay: null,
    byMonth: null,
    until: null,
    count: null,
    interval: 1,
  };

  const parts = rrulePart.split(";");

  for (const part of parts) {
    const [key, value] = part.split("=");
    if (!key || !value) continue;

    switch (key) {
      case "FREQ":
        if (["DAILY", "WEEKLY", "MONTHLY", "YEARLY"].includes(value)) {
          result.freq = value as RRuleFreq;
        }
        break;
      case "BYDAY":
        result.byDay = value.split(",");
        break;
      case "BYMONTHDAY":
        result.byMonthDay = value.split(",").map((item) => parseInt(item, 10));
        break;
      case "BYMONTH":
        result.byMonth = value.split(",").map((item) => parseInt(item, 10));
        break;
      case "UNTIL":
        result.until = value;
        break;
      case "COUNT":
        result.count = parseInt(value, 10);
        break;
      case "INTERVAL":
        result.interval = parseInt(value, 10);
        break;
    }
  }

  return result;
}

const DAY_INDEXES: Record<string, number> = {
  MO: 1,
  TU: 2,
  WE: 3,
  TH: 4,
  FR: 5,
  SA: 6,
  SU: 0,
};

function getLocalizedWeekdayName(code: string): string {
  const dayIndex = DAY_INDEXES[code];
  if (dayIndex === undefined) return code;

  const reference = new Date(2024, 0, 7 + dayIndex);
  return reference.toLocaleDateString(getLocaleConfig().locale, { weekday: "long" });
}

function formatRecurrence(parsed: ParsedRRule): string | undefined {
  if (!parsed.freq) return undefined;
  if (parsed.count !== null && parsed.count <= 1) return undefined;

  switch (parsed.freq) {
    case "DAILY": {
      if (
        parsed.byDay &&
        parsed.byDay.length === 5 &&
        parsed.byDay.includes("MO") &&
        parsed.byDay.includes("FR")
      ) {
        return translate("recurrenceWeekdays");
      }
      if (parsed.interval > 1) {
        return translate("recurrenceEveryDays", { count: parsed.interval });
      }
      return translate("recurrenceDaily");
    }

    case "WEEKLY": {
      if (parsed.byDay) {
        if (
          parsed.byDay.length === 5 &&
          ["MO", "TU", "WE", "TH", "FR"].every((day) => parsed.byDay?.includes(day))
        ) {
          return translate("recurrenceWeekdays");
        }
        if (parsed.byDay.length === 1) {
          const dayName = getLocalizedWeekdayName(parsed.byDay[0]);
          return translate("recurrenceEveryDay", { day: dayName });
        }
        const dayNames = parsed.byDay.map((dayCode) => getLocalizedWeekdayName(dayCode)).join(", ");
        return translate("recurrenceWeeklyDays", { days: dayNames });
      }
      if (parsed.interval > 1) {
        return translate("recurrenceEveryWeeks", { count: parsed.interval });
      }
      return translate("recurrenceWeekly");
    }

    case "MONTHLY": {
      if (parsed.byMonthDay) {
        const days = parsed.byMonthDay.join(", ");
        if (parsed.byMonth) {
          const months = parsed.byMonth
            .map((month) =>
              new Date(2000, month - 1).toLocaleDateString(getLocaleConfig().locale, { month: "short" }),
            )
            .join(", ");
          return translate("recurrenceMonthlyDaysMonths", { days, months });
        }
        return translate("recurrenceMonthlyDays", { days });
      }
      if (parsed.interval > 1) {
        return translate("recurrenceEveryMonths", { count: parsed.interval });
      }
      return translate("recurrenceMonthly");
    }

    case "YEARLY":
      return parsed.interval > 1
        ? translate("recurrenceEveryYears", { count: parsed.interval })
        : translate("recurrenceYearly");

    default:
      return undefined;
  }
}

function parseRRule(rruleStr: string): { dateTag: string; timeTag?: string; recurrenceTag?: string } | null {
  if (!rruleStr) return null;

  const dtstartMatch = rruleStr.match(/DTSTART:(\d{4})(\d{2})(\d{2})T?(\d{2})?(\d{2})?(\d{2})?/);
  if (!dtstartMatch) return null;

  const year = parseInt(dtstartMatch[1], 10);
  const month = parseInt(dtstartMatch[2], 10) - 1;
  const day = parseInt(dtstartMatch[3], 10);
  const hour = dtstartMatch[4] ? parseInt(dtstartMatch[4], 10) : null;
  const minute = dtstartMatch[5] ? parseInt(dtstartMatch[5], 10) : 0;

  const dtstart = new Date(year, month, day, hour ?? 0, minute);
  const { locale, hour12 } = getLocaleConfig();
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const tomorrow = new Date(today);
  tomorrow.setDate(tomorrow.getDate() + 1);

  let dateTag: string;
  if (dtstart.toDateString() === today.toDateString()) {
    dateTag = translate("today");
  } else if (dtstart.toDateString() === tomorrow.toDateString()) {
    dateTag = translate("tomorrow");
  } else {
    const daysDiff = Math.floor((dtstart.getTime() - today.getTime()) / (1000 * 60 * 60 * 24));
    if (daysDiff >= 0 && daysDiff < 7) {
      dateTag = dtstart.toLocaleDateString(locale, { weekday: "long" });
    } else {
      dateTag = dtstart.toLocaleDateString(locale, { month: "short", day: "numeric" });
    }
  }

  let timeTag: string | undefined;
  if (hour !== null) {
    timeTag = dtstart.toLocaleTimeString(locale, {
      hour: "numeric",
      minute: "2-digit",
      hour12,
    });
  }

  let recurrenceTag: string | undefined;
  const rruleMatch = rruleStr.match(/RRULE:(.+)/);
  if (rruleMatch) {
    const parsed = parseRRuleComponent(rruleMatch[1]);
    recurrenceTag = formatRecurrence(parsed);
  }

  return { dateTag, timeTag, recurrenceTag };
}

function parseScheduledTimeIso(scheduledTimeIso: string): { dateTag: string; timeTag?: string } | null {
  if (!scheduledTimeIso) return null;

  const scheduledDate = new Date(scheduledTimeIso);
  if (Number.isNaN(scheduledDate.getTime())) return null;

  const { locale, hour12 } = getLocaleConfig();
  const today = new Date();
  const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const startOfScheduled = new Date(
    scheduledDate.getFullYear(),
    scheduledDate.getMonth(),
    scheduledDate.getDate(),
  );
  const tomorrow = new Date(startOfToday);
  tomorrow.setDate(tomorrow.getDate() + 1);

  let dateTag: string;
  if (startOfScheduled.getTime() === startOfToday.getTime()) {
    dateTag = translate("today");
  } else if (startOfScheduled.getTime() === tomorrow.getTime()) {
    dateTag = translate("tomorrow");
  } else {
    const daysDiff = Math.floor((startOfScheduled.getTime() - startOfToday.getTime()) / (1000 * 60 * 60 * 24));
    if (daysDiff >= 0 && daysDiff < 7) {
      dateTag = scheduledDate.toLocaleDateString(locale, { weekday: "long" });
    } else {
      dateTag = scheduledDate.toLocaleDateString(locale, { month: "short", day: "numeric" });
    }
  }

  const timeTag = scheduledDate.toLocaleTimeString(locale, {
    hour: "numeric",
    minute: "2-digit",
    hour12,
  });

  return { dateTag, timeTag };
}

function formatDateTagOnly(date: Date): { dateTag: string } {
  const { locale } = getLocaleConfig();
  const today = new Date();
  const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const startOfDate = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  const tomorrow = new Date(startOfToday);
  tomorrow.setDate(tomorrow.getDate() + 1);

  let dateTag: string;
  if (startOfDate.getTime() === startOfToday.getTime()) {
    dateTag = translate("today");
  } else if (startOfDate.getTime() === tomorrow.getTime()) {
    dateTag = translate("tomorrow");
  } else {
    const daysDiff = Math.floor((startOfDate.getTime() - startOfToday.getTime()) / (1000 * 60 * 60 * 24));
    if (daysDiff >= 0 && daysDiff < 7) {
      dateTag = date.toLocaleDateString(locale, { weekday: "long" });
    } else {
      dateTag = date.toLocaleDateString(locale, { month: "short", day: "numeric" });
    }
  }

  return { dateTag };
}

function isRelativeDepartureCommute(payload: any): boolean {
  return Boolean(payload && payload.departure_time?.departure_type === "relative_to_arrival");
}

function getScheduleDateFromFields(
  scheduledTimeIso?: string,
  rrule?: string,
  legacyScheduledTime?: string,
): Date | null {
  if (scheduledTimeIso) {
    const parsed = new Date(scheduledTimeIso);
    if (!Number.isNaN(parsed.getTime())) return parsed;
  }

  if (rrule) {
    return parseRRuleDate(rrule);
  }

  if (legacyScheduledTime) {
    return parseLegacyScheduledTime(legacyScheduledTime);
  }

  return null;
}

function parseDurationTextMinutes(value?: string): number | undefined {
  if (!value) return undefined;

  const normalized = value.toLowerCase();
  const hourMatch = normalized.match(/(\d+)\s*h/);
  const minuteMatch = normalized.match(/(\d+)\s*m/);
  const hours = hourMatch ? parseInt(hourMatch[1], 10) : 0;
  const minutes = minuteMatch ? parseInt(minuteMatch[1], 10) : 0;
  const total = hours * 60 + minutes;

  return total > 0 ? total : undefined;
}

function getDirectionsDurationMinutes(directions: any): number | undefined {
  if (!directions || typeof directions !== "object") return undefined;
  if (typeof directions.total_duration_minutes === "number" && directions.total_duration_minutes > 0) {
    return directions.total_duration_minutes;
  }

  return parseDurationTextMinutes(
    typeof directions.total_duration === "string" ? directions.total_duration : undefined,
  );
}

function getCommuteArrivalDate(payload: any): Date | null {
  if (!payload) return null;

  return getScheduleDateFromFields(
    typeof payload.scheduled_time_iso === "string" ? payload.scheduled_time_iso : undefined,
    typeof payload.rrule === "string" ? payload.rrule : undefined,
    typeof payload.scheduled_time === "string" ? payload.scheduled_time : undefined,
  );
}

function getCommuteDepartureDate(payload: any, options?: { fallbackToArrival?: boolean }): Date | null {
  if (!payload || payload.departure_time?.departure_type !== "relative_to_arrival") {
    return null;
  }

  const arrivalDate = getCommuteArrivalDate(payload);
  if (!arrivalDate) return null;

  const routeMinutes = getDirectionsDurationMinutes(payload.directions);
  if (typeof routeMinutes !== "number") {
    return options?.fallbackToArrival ? arrivalDate : null;
  }

  const bufferMinutes = typeof payload.departure_time.buffer_minutes === "number"
    ? payload.departure_time.buffer_minutes
    : 0;

  return new Date(arrivalDate.getTime() - (routeMinutes + bufferMinutes) * 60_000);
}

function getDayLabelFromDate(date: Date): string {
  const { locale } = getLocaleConfig();
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const tomorrow = new Date(today);
  tomorrow.setDate(tomorrow.getDate() + 1);

  if (date.toDateString() === today.toDateString()) return translate("today");
  if (date.toDateString() === tomorrow.toDateString()) return translate("tomorrow");

  const daysDiff = Math.floor((date.getTime() - today.getTime()) / (1000 * 60 * 60 * 24));
  if (daysDiff >= 0 && daysDiff < 7) {
    return date.toLocaleDateString(locale, { weekday: "long" });
  }
  return date.toLocaleDateString(locale, { month: "short", day: "numeric" });
}

function getStartOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function parseRRuleDate(rruleStr: string): Date | null {
  const dtstartMatch = rruleStr.match(/DTSTART:(\d{4})(\d{2})(\d{2})T?(\d{2})?(\d{2})?(\d{2})?/);
  if (!dtstartMatch) return null;

  const year = parseInt(dtstartMatch[1], 10);
  const month = parseInt(dtstartMatch[2], 10) - 1;
  const day = parseInt(dtstartMatch[3], 10);
  const hour = dtstartMatch[4] ? parseInt(dtstartMatch[4], 10) : 0;
  const minute = dtstartMatch[5] ? parseInt(dtstartMatch[5], 10) : 0;
  const second = dtstartMatch[6] ? parseInt(dtstartMatch[6], 10) : 0;

  return new Date(year, month, day, hour, minute, second);
}

function parseLegacyScheduledTime(scheduledTime: string): Date | null {
  const trimmed = scheduledTime.trim();
  if (!trimmed) return null;

  if (/^\d{4}-\d{2}-\d{2}/.test(trimmed)) {
    const parsed = new Date(trimmed);
    return Number.isNaN(parsed.getTime()) ? null : parsed;
  }

  return null;
}

export function getSharedSchedule(payload: any): {
  scheduledTimeIso?: string;
  rrule?: string;
  durationMinutes?: number;
} {
  if (!payload || typeof payload !== "object") {
    return {};
  }

  const departure = payload.departure_time;
  if (departure && typeof departure === "object" && departure.departure_type === "fixed") {
    return {
      scheduledTimeIso: typeof departure.departure_time_iso === "string" ? departure.departure_time_iso : undefined,
      rrule: typeof departure.departure_rrule === "string" ? departure.departure_rrule : undefined,
      durationMinutes: undefined,
    };
  }

  return {
    scheduledTimeIso: typeof payload.scheduled_time_iso === "string" ? payload.scheduled_time_iso : undefined,
    rrule: typeof payload.rrule === "string" ? payload.rrule : undefined,
    durationMinutes: typeof payload.duration_minutes === "number" ? payload.duration_minutes : undefined,
  };
}

export function getScheduleDate(payload: any): Date | null {
  const relativeDeparture = getCommuteDepartureDate(payload, { fallbackToArrival: true });
  if (relativeDeparture) return relativeDeparture;

  const schedule = getSharedSchedule(payload);
  return getScheduleDateFromFields(schedule.scheduledTimeIso, schedule.rrule, payload?.scheduled_time);
}

export function getDisplayScheduleDate(payload: any): Date | null {
  if (isRelativeDepartureCommute(payload)) {
    return getCommuteDepartureDate(payload, { fallbackToArrival: false });
  }

  const relativeDeparture = getCommuteDepartureDate(payload, { fallbackToArrival: false });
  if (relativeDeparture) return relativeDeparture;

  const schedule = getSharedSchedule(payload);
  return getScheduleDateFromFields(schedule.scheduledTimeIso, schedule.rrule, payload?.scheduled_time);
}

export function getScheduleTags(payload: any): { dateTag: string; timeTag?: string; recurrenceTag?: string } | null {
  const displayDate = getDisplayScheduleDate(payload);
  if (displayDate) {
    return parseScheduledTimeIso(displayDate.toISOString());
  }

  if (isRelativeDepartureCommute(payload)) {
    const arrivalDate = getCommuteArrivalDate(payload);
    return arrivalDate ? formatDateTagOnly(arrivalDate) : null;
  }

  const schedule = getSharedSchedule(payload);
  if (schedule.scheduledTimeIso) {
    return parseScheduledTimeIso(schedule.scheduledTimeIso);
  }
  if (schedule.rrule) {
    return parseRRule(schedule.rrule);
  }

  return null;
}

function getScheduleSortKey(payload: any): string {
  const schedule = getSharedSchedule(payload);
  if (schedule.scheduledTimeIso) return schedule.scheduledTimeIso;
  if (schedule.rrule) return schedule.rrule;
  return payload?.scheduled_time || "";
}

function compareRelatedCommuteOrdering(left: TicketModel, right: TicketModel): number {
  const leftRelatedEventId = left.type === "COMMUTE" && typeof left.payload?.related_event_id === "string"
    ? left.payload.related_event_id
    : undefined;
  const rightRelatedEventId = right.type === "COMMUTE" && typeof right.payload?.related_event_id === "string"
    ? right.payload.related_event_id
    : undefined;

  if (left.type === "COMMUTE" && right.id === leftRelatedEventId) return -1;
  if (right.type === "COMMUTE" && left.id === rightRelatedEventId) return 1;
  return 0;
}

export function compareScheduledTasks(left: TicketModel, right: TicketModel): number {
  const leftDate = getScheduleDate(left.payload);
  const rightDate = getScheduleDate(right.payload);

  if (leftDate && rightDate) {
    const diff = leftDate.getTime() - rightDate.getTime();
    if (diff !== 0) return diff;
  } else if (leftDate) {
    return -1;
  } else if (rightDate) {
    return 1;
  }

  const sortKeyDiff = getScheduleSortKey(left.payload).localeCompare(getScheduleSortKey(right.payload));
  if (sortKeyDiff !== 0) return sortKeyDiff;

  const relatedDiff = compareRelatedCommuteOrdering(left, right);
  if (relatedDiff !== 0) return relatedDiff;

  const titleDiff = (left.payload?.title || "").localeCompare(right.payload?.title || "");
  if (titleDiff !== 0) return titleDiff;

  return left.id.localeCompare(right.id);
}

export function formatAbsoluteSchedule(date: Date): string {
  const { locale, hour12 } = getLocaleConfig();
  return date.toLocaleString(locale, {
    weekday: "long",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    hour12,
  });
}

export function formatDurationMinutes(durationMinutes: number): string {
  if (durationMinutes < 60) return `${durationMinutes} min`;
  const hours = Math.floor(durationMinutes / 60);
  const minutes = durationMinutes % 60;
  if (minutes === 0) return `${hours}h`;

  return `${hours}h ${minutes}m`;
}

export function formatMetadataTag(value: string): string {
  return value
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function resolveStructuredLocation(
  location: any,
  savedLocations: SavedLocationIndex = {},
): { display?: string; query?: string } {
  if (!location || typeof location !== "object") return {};

  switch (location.location_type) {
    case "saved_location": {
      const saved = typeof location.location_id === "string" ? savedLocations[location.location_id] : undefined;
      if (!saved) {
        const fallback = typeof location.label === "string" ? location.label : location.location_id;
        return { display: fallback, query: fallback };
      }

      const resolved = resolveStructuredLocation(saved.location, savedLocations);
      return {
        display: saved.label || resolved?.display,
        query: resolved?.query ?? saved.label,
      };
    }
    case "address_text":
      return typeof location.address === "string"
        ? { display: location.label || location.address, query: location.address }
        : {};
    case "coordinates": {
      const latitude = typeof location.latitude === "number" ? location.latitude : undefined;
      const longitude = typeof location.longitude === "number" ? location.longitude : undefined;
      if (typeof latitude === "number" && typeof longitude === "number") {
        const fallback = `${latitude}, ${longitude}`;
        return {
          display: typeof location.label === "string" && location.label.trim()
            ? `${location.label} (${latitude}, ${longitude})`
            : fallback,
          query: fallback,
        };
      }
      return typeof location.label === "string" ? { display: location.label, query: location.label } : {};
    }
    case "place_id":
      return typeof location.label === "string"
        ? { display: location.label, query: location.label }
        : typeof location.place_id === "string"
          ? { display: location.place_id, query: location.place_id }
          : {};
    case "current_position":
      return {
        display: typeof location.label === "string" && location.label.trim()
          ? location.label
          : "Current position",
        query: typeof location.label === "string" && location.label.trim()
          ? location.label
          : "Current position",
      };
    default:
      return {};
  }
}

export function buildGoogleMapsUrl(origin?: string, destination?: string): string | undefined {
  if (origin && destination) {
    return `https://www.google.com/maps/dir/?api=1&origin=${encodeURIComponent(origin)}&destination=${encodeURIComponent(destination)}`;
  }

  if (destination) {
    return `https://www.google.com/maps/search/?api=1&query=${encodeURIComponent(destination)}`;
  }

  return undefined;
}

export function buildAppleMapsUrl(origin?: string, destination?: string): string | undefined {
  if (origin && destination) {
    return `https://maps.apple.com/?saddr=${encodeURIComponent(origin)}&daddr=${encodeURIComponent(destination)}`;
  }

  if (destination) {
    return `https://maps.apple.com/?q=${encodeURIComponent(destination)}`;
  }

  return undefined;
}

export function buildGoogleMapsEmbedUrl(origin?: string, destination?: string): string | undefined {
  if (origin && destination) {
    return `https://www.google.com/maps?output=embed&saddr=${encodeURIComponent(origin)}&daddr=${encodeURIComponent(destination)}`;
  }

  if (destination) {
    return `https://www.google.com/maps?output=embed&q=${encodeURIComponent(destination)}`;
  }

  return undefined;
}

export function groupTickets(tickets: TicketModel[]) {
  const grouped: {
    inFocus: TicketModel | null;
    days: Record<string, { sortDate: Date; tickets: TicketModel[] }>;
    unplanned: TicketModel[];
  } = { inFocus: null, days: {}, unplanned: [] };

  for (const ticket of tickets) {
    if (ticket.status === "in_focus" && !grouped.inFocus) {
      grouped.inFocus = ticket;
      break;
    }
  }

  const regularTickets = tickets.filter((ticket) => ticket.status !== "in_focus");

  for (const ticket of regularTickets) {
    const payload = ticket.payload || {};
    const scheduledDate = getScheduleDate(payload);

    if (scheduledDate) {
      const dayLabel = getDayLabelFromDate(scheduledDate);
      if (!grouped.days[dayLabel]) {
        grouped.days[dayLabel] = { sortDate: getStartOfDay(scheduledDate), tickets: [] };
      }
      grouped.days[dayLabel].tickets.push(ticket);
      continue;
    }

    const time = payload.scheduled_time?.trim();
    if (!time) {
      grouped.unplanned.push(ticket);
      continue;
    }

    const lowerTime = time.toLowerCase();
    let dayLabel = translate("today");

    if (lowerTime.includes("tomorrow")) {
      dayLabel = translate("tomorrow");
    } else {
      const matchedWeekday = LEGACY_WEEKDAY_LABELS.find((entry) => lowerTime.includes(entry.token));
      if (matchedWeekday) {
        const reference = new Date(2024, 0, 7 + matchedWeekday.dayIndex);
        dayLabel = reference.toLocaleDateString(getLocaleConfig().locale, { weekday: "long" });
      } else if (/^\d{4}-\d{2}-\d{2}/.test(time)) {
        const date = new Date(time);
        if (!Number.isNaN(date.getTime())) {
          dayLabel = date.toLocaleDateString(getLocaleConfig().locale, { weekday: "long" });
        }
      }
    }

    if (!grouped.days[dayLabel]) {
      grouped.days[dayLabel] = { sortDate: new Date(8640000000000000), tickets: [] };
    }
    grouped.days[dayLabel].tickets.push(ticket);
  }

  const orderedDays: Record<string, TicketModel[]> = {};
  Object.entries(grouped.days)
    .sort(([, left], [, right]) => left.sortDate.getTime() - right.sortDate.getTime())
    .forEach(([label, entry]) => {
      entry.tickets.sort(compareScheduledTasks);
      orderedDays[label] = entry.tickets;
    });

  grouped.unplanned.sort((left, right) => {
    const titleLeft = left.payload?.title || "";
    const titleRight = right.payload?.title || "";
    return titleLeft.localeCompare(titleRight);
  });

  return {
    inFocus: grouped.inFocus,
    days: orderedDays,
    unplanned: grouped.unplanned,
  };
}

export interface SyncAction {
  type: "CREATE" | "UPDATE" | "DELETE";
  entity_id: string;
  entity_type: string;
  payload?: any;
  status?: string;
}

export function projectTickets(baseTickets: TicketModel[], actions: SyncAction[]): TicketModel[] {
  let projected = [...baseTickets];
  
  for (const action of actions) {
    switch (action.type) {
      case "CREATE": {
        // Only add if not already there
        if (!projected.find(t => t.id === action.entity_id)) {
          projected.push({
            id: action.entity_id,
            type: action.entity_type as any,
            status: (action.status as any) || "idle",
            title: action.payload?.title || "Untitled",
            payload: action.payload || {},
            updated_at: new Date().toISOString(),
          });
        }
        break;
      }
      case "UPDATE": {
        projected = projected.map(t => {
          if (t.id === action.entity_id) {
            return {
              ...t,
              status: (action.status as any) || t.status,
              payload: { ...t.payload, ...action.payload },
              updated_at: new Date().toISOString(),
            };
          }
          return t;
        });
        break;
      }
      case "DELETE": {
        projected = projected.filter(t => t.id !== action.entity_id);
        break;
      }
    }
  }
  
  return projected;
}