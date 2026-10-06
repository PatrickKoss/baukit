FROM quay.io/keycloak/keycloak:26.8.0
ENV JAVA_OPTS_KC_HEAP="-Xms128m -Xmx512m" JAVA_OPTS_APPEND="-XX:ActiveProcessorCount=2"
RUN /opt/keycloak/bin/kc.sh build --db=dev-file
