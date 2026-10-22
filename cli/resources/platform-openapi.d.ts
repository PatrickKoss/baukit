
export interface paths {
    "/platform/inbound": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Receive a signed hub event
         * @description Authenticates raw JSON using the X-Suite signature headers. Duplicate and unknown events are acknowledged. Unavailable handlers or storage return 503 for retry.
         */
        post: operations["platform_inbound"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}

export interface operations {
    platform_inbound: {
        parameters: {
            query?: never;
            header: {
                /** @description Configured hub source */
                "x-suite-source": string;
                /** @description Canonical Unix seconds within 300 seconds */
                "x-suite-timestamp": string;
                /** @description Delivery attempt ID */
                "x-suite-delivery-id": string;
                /** @description HMAC-SHA256 signature */
                "x-suite-signature": string;
            };
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": unknown;
            };
        };
        responses: {
            /** @description Applied, duplicate or unknown event */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Invalid signature headers */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Event ID reused with changed bytes */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Body too large */
            413: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description JSON content type required */
            415: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Invalid event */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Retry after handler or storage failure */
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
}
