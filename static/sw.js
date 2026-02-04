// CENTRABIO R&D NEXUS - Service Worker for Offline Support
// Version: 1.0.0

const CACHE_NAME = 'centrabio-nexus-v1';
const STATIC_CACHE = 'centrabio-static-v1';
const DATA_CACHE = 'centrabio-data-v1';

// Static assets to cache immediately
const STATIC_ASSETS = [
    '/',
    '/index.html',
    '/manifest.json',
    '/offline.html',
    'https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap',
    'https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.5.1/css/all.min.css',
    'https://unpkg.com/alpinejs@3.x.x/dist/cdn.min.js',
    'https://cdn.jsdelivr.net/npm/chart.js'
];

// API endpoints to cache
const API_CACHE_ENDPOINTS = [
    '/api/v1/projects',
    '/api/v1/formulas',
    '/api/v1/monitoring/sessions'
];

// Install event - cache static assets
self.addEventListener('install', (event) => {
    console.log('[Service Worker] Installing...');
    
    event.waitUntil(
        caches.open(STATIC_CACHE)
            .then((cache) => {
                console.log('[Service Worker] Caching static assets');
                return cache.addAll(STATIC_ASSETS);
            })
            .then(() => {
                console.log('[Service Worker] Static assets cached');
                return self.skipWaiting();
            })
            .catch((error) => {
                console.error('[Service Worker] Failed to cache static assets:', error);
            })
    );
});

// Activate event - clean up old caches
self.addEventListener('activate', (event) => {
    console.log('[Service Worker] Activating...');
    
    event.waitUntil(
        caches.keys()
            .then((cacheNames) => {
                return Promise.all(
                    cacheNames
                        .filter((name) => {
                            return name !== STATIC_CACHE && name !== DATA_CACHE;
                        })
                        .map((name) => {
                            console.log('[Service Worker] Deleting old cache:', name);
                            return caches.delete(name);
                        })
                );
            })
            .then(() => {
                console.log('[Service Worker] Activated');
                return self.clients.claim();
            })
    );
});

// Fetch event - serve from cache, fallback to network
self.addEventListener('fetch', (event) => {
    const url = new URL(event.request.url);
    
    // Skip non-GET requests
    if (event.request.method !== 'GET') {
        // For POST/PUT/DELETE, try to queue for later sync if offline
        if (!navigator.onLine) {
            event.respondWith(handleOfflinePost(event.request));
        }
        return;
    }
    
    // API requests - network first, cache fallback
    if (url.pathname.startsWith('/api/')) {
        event.respondWith(networkFirstStrategy(event.request));
        return;
    }
    
    // Static assets - cache first, network fallback
    event.respondWith(cacheFirstStrategy(event.request));
});

// Cache-first strategy for static assets
async function cacheFirstStrategy(request) {
    try {
        const cachedResponse = await caches.match(request);
        
        if (cachedResponse) {
            // Refresh cache in background
            fetchAndCache(request, STATIC_CACHE);
            return cachedResponse;
        }
        
        const networkResponse = await fetch(request);
        
        // Cache the response
        if (networkResponse.ok) {
            const cache = await caches.open(STATIC_CACHE);
            cache.put(request, networkResponse.clone());
        }
        
        return networkResponse;
    } catch (error) {
        console.error('[Service Worker] Cache-first failed:', error);
        
        // Return offline page for navigation requests
        if (request.mode === 'navigate') {
            const offlinePage = await caches.match('/offline.html');
            if (offlinePage) return offlinePage;
        }
        
        return new Response('Offline', { status: 503, statusText: 'Service Unavailable' });
    }
}

// Network-first strategy for API requests
async function networkFirstStrategy(request) {
    try {
        const networkResponse = await fetch(request);
        
        // Cache successful GET responses
        if (networkResponse.ok && request.method === 'GET') {
            const cache = await caches.open(DATA_CACHE);
            cache.put(request, networkResponse.clone());
        }
        
        return networkResponse;
    } catch (error) {
        console.log('[Service Worker] Network failed, trying cache:', request.url);
        
        const cachedResponse = await caches.match(request);
        
        if (cachedResponse) {
            // Add header to indicate cached response
            const headers = new Headers(cachedResponse.headers);
            headers.set('X-Cached-Response', 'true');
            
            return new Response(cachedResponse.body, {
                status: cachedResponse.status,
                statusText: cachedResponse.statusText,
                headers: headers
            });
        }
        
        // Return empty data response for API endpoints
        return new Response(JSON.stringify({
            success: false,
            message: 'You are offline. Data will be synced when connection is restored.',
            offline: true,
            data: []
        }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' }
        });
    }
}

// Handle offline POST/PUT/DELETE requests
async function handleOfflinePost(request) {
    // Queue the request for later sync
    try {
        const clonedRequest = request.clone();
        const body = await clonedRequest.json();
        
        // Store in IndexedDB for later sync
        await queueForSync({
            url: request.url,
            method: request.method,
            body: body,
            timestamp: Date.now()
        });
        
        return new Response(JSON.stringify({
            success: true,
            message: 'Request queued for sync when online',
            queued: true
        }), {
            status: 202,
            headers: { 'Content-Type': 'application/json' }
        });
    } catch (error) {
        return new Response(JSON.stringify({
            success: false,
            message: 'Failed to queue request for offline sync'
        }), {
            status: 500,
            headers: { 'Content-Type': 'application/json' }
        });
    }
}

// Fetch and cache in background
async function fetchAndCache(request, cacheName) {
    try {
        const response = await fetch(request);
        if (response.ok) {
            const cache = await caches.open(cacheName);
            cache.put(request, response);
        }
    } catch (error) {
        // Silently fail - just refreshing cache
    }
}

// Queue request for later sync (using IndexedDB)
function queueForSync(requestData) {
    return new Promise((resolve, reject) => {
        const dbRequest = indexedDB.open('CentraBioOfflineDB', 1);
        
        dbRequest.onupgradeneeded = (event) => {
            const db = event.target.result;
            if (!db.objectStoreNames.contains('pendingRequests')) {
                db.createObjectStore('pendingRequests', { keyPath: 'timestamp' });
            }
        };
        
        dbRequest.onsuccess = (event) => {
            const db = event.target.result;
            const transaction = db.transaction(['pendingRequests'], 'readwrite');
            const store = transaction.objectStore('pendingRequests');
            
            store.add(requestData);
            
            transaction.oncomplete = () => resolve();
            transaction.onerror = () => reject(transaction.error);
        };
        
        dbRequest.onerror = () => reject(dbRequest.error);
    });
}

// Background sync event
self.addEventListener('sync', (event) => {
    console.log('[Service Worker] Sync event triggered:', event.tag);
    
    if (event.tag === 'sync-pending-requests') {
        event.waitUntil(syncPendingRequests());
    }
});

// Sync all pending requests
async function syncPendingRequests() {
    return new Promise((resolve, reject) => {
        const dbRequest = indexedDB.open('CentraBioOfflineDB', 1);
        
        dbRequest.onsuccess = async (event) => {
            const db = event.target.result;
            const transaction = db.transaction(['pendingRequests'], 'readwrite');
            const store = transaction.objectStore('pendingRequests');
            
            const getAllRequest = store.getAll();
            
            getAllRequest.onsuccess = async () => {
                const pendingRequests = getAllRequest.result;
                
                for (const request of pendingRequests) {
                    try {
                        const response = await fetch(request.url, {
                            method: request.method,
                            headers: {
                                'Content-Type': 'application/json',
                                'Authorization': `Bearer ${await getStoredToken()}`
                            },
                            body: JSON.stringify(request.body)
                        });
                        
                        if (response.ok) {
                            // Remove from queue
                            store.delete(request.timestamp);
                        }
                    } catch (error) {
                        console.error('[Service Worker] Failed to sync request:', error);
                    }
                }
                
                resolve();
            };
        };
        
        dbRequest.onerror = () => reject(dbRequest.error);
    });
}

// Get stored auth token
async function getStoredToken() {
    // Tokens are stored in localStorage by the main app
    // Service worker can't access localStorage directly, so we use a message
    const clients = await self.clients.matchAll();
    if (clients.length > 0) {
        return new Promise((resolve) => {
            const messageChannel = new MessageChannel();
            messageChannel.port1.onmessage = (event) => {
                resolve(event.data.token);
            };
            clients[0].postMessage({ type: 'GET_TOKEN' }, [messageChannel.port2]);
        });
    }
    return null;
}

// Push notification handling
self.addEventListener('push', (event) => {
    console.log('[Service Worker] Push received:', event);
    
    let data = {};
    
    if (event.data) {
        try {
            data = event.data.json();
        } catch (e) {
            data = { title: 'Notification', body: event.data.text() };
        }
    }
    
    const options = {
        body: data.body || 'New update available',
        icon: '/icons/icon-192.png',
        badge: '/icons/badge-72.png',
        vibrate: [100, 50, 100],
        data: data.data || {},
        actions: data.actions || [
            { action: 'view', title: 'View' },
            { action: 'dismiss', title: 'Dismiss' }
        ]
    };
    
    event.waitUntil(
        self.registration.showNotification(data.title || 'CENTRABIO NEXUS', options)
    );
});

// Notification click handling
self.addEventListener('notificationclick', (event) => {
    console.log('[Service Worker] Notification click:', event);
    
    event.notification.close();
    
    if (event.action === 'view' || !event.action) {
        const urlToOpen = event.notification.data?.url || '/';
        
        event.waitUntil(
            self.clients.matchAll({ type: 'window', includeUncontrolled: true })
                .then((clientList) => {
                    // Check if there's already a window open
                    for (const client of clientList) {
                        if (client.url.includes(self.registration.scope)) {
                            return client.focus();
                        }
                    }
                    // Open new window
                    return self.clients.openWindow(urlToOpen);
                })
        );
    }
});

// Message handling from main app
self.addEventListener('message', (event) => {
    console.log('[Service Worker] Message received:', event.data);
    
    if (event.data.type === 'SKIP_WAITING') {
        self.skipWaiting();
    }
    
    if (event.data.type === 'GET_TOKEN') {
        event.ports[0].postMessage({ token: event.data.token });
    }
    
    if (event.data.type === 'CACHE_URLS') {
        event.waitUntil(
            caches.open(DATA_CACHE)
                .then((cache) => cache.addAll(event.data.urls))
        );
    }
    
    if (event.data.type === 'CLEAR_CACHE') {
        event.waitUntil(
            caches.keys().then((names) => {
                return Promise.all(names.map((name) => caches.delete(name)));
            })
        );
    }
});

console.log('[Service Worker] Script loaded');
