.PHONY: qa-setup qa-android-setup qa-ios-setup qa-android-build qa-ios-build qa-android qa-ios e2e-mobile e2e-mobile-live e2e-android e2e-android-live e2e-ios e2e-ios-live qa-android-down qa-ios-down qa-down

qa-setup: qa-android-setup

qa-android-setup:
	bash scripts/qa/android-sdk.sh

qa-ios-setup:
	bash scripts/qa/ios-simulator.sh prepare

qa-android-build: qa-android-setup
	bash scripts/qa/build-android.sh

qa-ios-build: qa-ios-setup
	bash scripts/qa/build-ios.sh

qa-android: qa-android-setup
	bash scripts/qa/android-env.sh start

qa-ios: qa-ios-setup
	bash scripts/qa/ios-env.sh start

e2e-android-live:
	bash scripts/qa/run-maestro.sh android

e2e-ios-live:
	bash scripts/qa/run-maestro.sh ios

e2e-mobile-live: e2e-android-live

e2e-android:
	@set -eu; \
		cleanup() { $(MAKE) --no-print-directory qa-android-down; }; \
		trap cleanup EXIT INT TERM; \
		$(MAKE) --no-print-directory qa-android; \
		$(MAKE) --no-print-directory e2e-android-live

e2e-ios:
	@set -eu; \
		cleanup() { $(MAKE) --no-print-directory qa-ios-down; }; \
		trap cleanup EXIT INT TERM; \
		$(MAKE) --no-print-directory qa-ios; \
		$(MAKE) --no-print-directory e2e-ios-live

e2e-mobile: e2e-android

qa-android-down:
	bash scripts/qa/android-env.sh stop

qa-ios-down:
	bash scripts/qa/ios-env.sh stop

qa-down:
	@bash scripts/qa/android-env.sh stop
	@bash scripts/qa/ios-env.sh stop
