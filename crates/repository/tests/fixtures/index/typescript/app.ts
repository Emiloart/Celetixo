import { base as increment } from './math';
export function app(): number { return increment(2); }
export function test_app(): boolean { return app() === 3; }
