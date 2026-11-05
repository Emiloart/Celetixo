import { base } from './math';
export function app() { return base(2); }
export function View() { return <p>{app()}</p>; }
