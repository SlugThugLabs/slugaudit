import { Widget } from './widget';

export function main(): void {
    const widget = new Widget('demo', 3);
    console.log(widget.scaled(2));
}
