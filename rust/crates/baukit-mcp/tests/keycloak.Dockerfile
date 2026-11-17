FROM quay.io/keycloak/keycloak:26.8.2@sha256:c68e286cacd87b942ac1939a0a1481461109d6acd3a57dc496969a319a76bbf0
ENV JAVA_OPTS_KC_HEAP="-Xms128m -Xmx512m" JAVA_OPTS_APPEND="-XX:ActiveProcessorCount=2"
RUN /opt/keycloak/bin/kc.sh build --db=dev-file
