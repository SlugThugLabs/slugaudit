import fs from 'fs';

export class Widget {
    constructor(public name: string, public value: number = 0) {}

    scaled(factor: number): number {
        return this.value * factor;
    }
}
