import { helperValue } from './helper.js';
import fs from 'fs';

export function main() {
    return helperValue() + fs.existsSync('/tmp');
}
