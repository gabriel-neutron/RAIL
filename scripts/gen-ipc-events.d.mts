// Hand-written types for the generator so its unit test stays type-checked.

export interface ContractField {
  name: string;
  type: string;
  doc?: string[];
  optional?: boolean;
  nullable?: boolean;
  list?: boolean;
}

export interface ContractEvent {
  name: string;
  type: string;
  doc?: string[];
  fields: ContractField[];
}

export type ParsedEvent = ContractEvent & { key: string };

export function parseContract(contract: unknown): ParsedEvent[];
export function renderRust(events: ParsedEvent[]): string;
export function renderTs(events: ParsedEvent[]): string;
export function generate(): void;
